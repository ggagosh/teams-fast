//! Rich Graph notifications: the installation's RSA key and self-signed certificate, decryption of
//! `encryptedContent`, and validation-token checks. HTTP (signing keys) stays in `teams.rs`.
use aes::Aes256;
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use rsa::{
    BigUint, Oaep, RsaPrivateKey, RsaPublicKey,
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    signature::Verifier,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{str::FromStr, sync::OnceLock, time::Duration};
use teamsfast_relay::EncryptedContent;
use x509_cert::{
    builder::{Builder, CertificateBuilder, Profile},
    der::Encode,
    name::Name,
    serial_number::SerialNumber,
    spki::SubjectPublicKeyInfoOwned,
    time::Validity,
};

const SECRET: &str = "rich-notifications-key";
/// Microsoft Graph Change Tracking, the only caller allowed to sign rich notifications.
const GRAPH_CHANGE_TRACKING: &str = "0bf30f3b-4a52-48df-9a82-234910c4a086";

pub(crate) struct Keys {
    key: RsaPrivateKey,
    /// Base64 DER X.509 certificate for `encryptionCertificate`.
    pub certificate: String,
    /// `encryptionCertificateId`; derived from the public key, so it changes only with the key.
    pub id: String,
}

/// The installation's key, created once and kept with the other secrets. `None` disables rich
/// notifications (subscriptions then carry IDs only and the app fetches each message).
pub(crate) fn keys() -> Option<&'static Keys> {
    static KEYS: OnceLock<Option<Keys>> = OnceLock::new();
    KEYS.get_or_init(|| {
        load()
            .map_err(|error| {
                if crate::teams::tracing() {
                    eprintln!("TeamsFast rich: disabled ({error})");
                }
            })
            .ok()
    })
    .as_ref()
}

fn load() -> Result<Keys, String> {
    let saved = crate::settings::read_secret(SECRET)?
        .and_then(|value| STANDARD.decode(value).ok())
        .and_then(|der| RsaPrivateKey::from_pkcs8_der(&der).ok());
    let key = match saved {
        Some(key) => key,
        None => {
            let key = RsaPrivateKey::new(&mut rand_core::OsRng, 2048)
                .map_err(|_| "could not create a key")?;
            let der = key.to_pkcs8_der().map_err(|_| "could not encode the key")?;
            crate::settings::write_secret(SECRET, &STANDARD.encode(der.as_bytes()))?;
            key
        }
    };
    from_key(key)
}

fn from_key(key: RsaPrivateKey) -> Result<Keys, String> {
    let public = key.to_public_key();
    let spki = SubjectPublicKeyInfoOwned::from_key(public.clone())
        .map_err(|_| "could not encode the public key")?;
    let fingerprint = Sha256::digest(
        public
            .to_public_key_der()
            .map_err(|_| "could not encode the public key")?
            .as_bytes(),
    );
    // Graph checks neither issuer nor dates; a fresh self-signed certificate per launch never expires in use.
    let signer = SigningKey::<Sha256>::new(key.clone());
    let certificate = CertificateBuilder::new(
        Profile::Root,
        SerialNumber::from(1_u32),
        Validity::from_now(Duration::from_secs(365 * 24 * 60 * 60))
            .map_err(|_| "invalid validity")?,
        Name::from_str("CN=TeamsFast notifications").map_err(|_| "invalid name")?,
        spki,
        &signer,
    )
    .and_then(|builder| builder.build::<Signature>())
    .map_err(|_| "could not create the certificate")?
    .to_der()
    .map_err(|_| "could not encode the certificate")?;
    Ok(Keys {
        key,
        certificate: STANDARD.encode(certificate),
        id: format!(
            "teamsfast-{}",
            fingerprint[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
    })
}

impl Keys {
    /// Graph's scheme: RSA-OAEP(SHA-1) wraps a one-time AES-256 key; HMAC-SHA256 over the
    /// ciphertext authenticates it; AES-CBC/PKCS7 with the key's first 16 bytes as IV.
    pub fn decrypt(&self, content: &EncryptedContent) -> Result<Vec<u8>, &'static str> {
        if content.encryption_certificate_id != self.id {
            return Err("encrypted for another key");
        }
        let wrapped = STANDARD.decode(&content.data_key).map_err(|_| "bad key")?;
        let key = self
            .key
            .decrypt(Oaep::new::<sha1::Sha1>(), &wrapped)
            .map_err(|_| "could not unwrap the key")?;
        let data = STANDARD.decode(&content.data).map_err(|_| "bad data")?;
        let signature = STANDARD
            .decode(&content.data_signature)
            .map_err(|_| "bad signature")?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| "bad key")?;
        mac.update(&data);
        mac.verify_slice(&signature)
            .map_err(|_| "signature mismatch")?;
        if key.len() != 32 {
            return Err("unexpected key size");
        }
        cbc::Decryptor::<Aes256>::new_from_slices(&key, &key[..16])
            .map_err(|_| "bad key")?
            .decrypt_padded_vec_mut::<Pkcs7>(&data)
            .map_err(|_| "could not decrypt")
    }
}

/// An RSA signing key from Microsoft's JWKS (`n`/`e` are base64url).
pub(crate) fn public_key(n: &str, e: &str) -> Option<RsaPublicKey> {
    let n = URL_SAFE_NO_PAD.decode(n).ok()?;
    let e = URL_SAFE_NO_PAD.decode(e).ok()?;
    RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)).ok()
}

/// The token's key ID, used to pick the Microsoft signing key.
pub(crate) fn token_kid(token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Header {
        kid: String,
    }
    let header = URL_SAFE_NO_PAD.decode(token.split('.').next()?).ok()?;
    serde_json::from_slice::<Header>(&header)
        .ok()
        .map(|header| header.kid)
}

/// Graph's required checks for each validation token: Microsoft signature (RS256), not expired,
/// issued for this app, and called by Graph change tracking.
pub(crate) fn verify_token(
    token: &str,
    key: &RsaPublicKey,
    client_id: &str,
) -> Result<(), &'static str> {
    #[derive(Deserialize)]
    struct Header {
        alg: String,
    }
    #[derive(Deserialize)]
    struct Claims {
        aud: String,
        iss: String,
        exp: i64,
        nbf: Option<i64>,
        ver: Option<String>,
        appid: Option<String>,
        azp: Option<String>,
    }
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed token");
    };
    let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| "malformed token");
    let alg: Header = serde_json::from_slice(&decode(header)?).map_err(|_| "malformed token")?;
    if alg.alg != "RS256" {
        return Err("unexpected token algorithm");
    }
    let signature = Signature::try_from(decode(signature)?.as_slice()).map_err(|_| "bad token")?;
    VerifyingKey::<Sha256>::new(key.clone())
        .verify(
            &token.as_bytes()[..header.len() + 1 + payload.len()],
            &signature,
        )
        .map_err(|_| "token signature mismatch")?;
    let claims: Claims =
        serde_json::from_slice(&decode(payload)?).map_err(|_| "malformed token")?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let skew = 5 * 60;
    let caller = if claims.ver.as_deref() == Some("2.0") {
        claims.azp
    } else {
        claims.appid
    };
    if claims.exp + skew < now
        || claims.nbf.is_some_and(|nbf| nbf - skew > now)
        || !claims.aud.eq_ignore_ascii_case(client_id)
        || caller.as_deref() != Some(GRAPH_CHANGE_TRACKING)
        || !(claims.iss.starts_with("https://sts.windows.net/")
            || claims.iss.starts_with("https://login.microsoftonline.com/"))
    {
        return Err("token claims rejected");
    }
    Ok(())
}
