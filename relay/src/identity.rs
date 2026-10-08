//! Microsoft ID-token check for registration, so the desktop needs no shared relay secret.
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) struct Identity {
    client_id: String,
    tenant: Option<String>,
    keys: Mutex<Option<(Instant, HashMap<String, DecodingKey>)>>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    tid: String,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    n: Option<String>,
    e: Option<String>,
}

impl Identity {
    pub fn new(client_id: &str, tenant: Option<&str>) -> Self {
        Self {
            client_id: client_id.into(),
            tenant: tenant.map(str::to_owned),
            keys: Mutex::new(None),
        }
    }

    /// A v2.0 ID token signed by Microsoft for this app (and tenant, when configured).
    pub async fn verify(&self, token: &str) -> bool {
        let Some(kid) = decode_header(token).ok().and_then(|header| header.kid) else {
            return false;
        };
        let Some(key) = self.key(&kid).await else {
            return false;
        };
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.client_id]);
        validation.leeway = 300;
        let data = match decode::<Claims>(token, &key, &validation) {
            Ok(data) => data,
            Err(error) => {
                // The error kind only (expired, audience, signature…); never the token.
                eprintln!("TeamsFast relay: ID token rejected: {:?}", error.kind());
                return false;
            }
        };
        let claims = data.claims;
        claims.iss == format!("https://login.microsoftonline.com/{}/v2.0", claims.tid)
            && self
                .tenant
                .as_ref()
                .is_none_or(|tenant| tenant.eq_ignore_ascii_case(&claims.tid))
    }

    /// Cached signing keys; an unknown key ID refetches at most once a minute (keys rotate).
    async fn key(&self, kid: &str) -> Option<DecodingKey> {
        if let Ok(cache) = self.keys.lock()
            && let Some((fetched, keys)) = &*cache
        {
            if let Some(key) = keys.get(kid) {
                return Some(key.clone());
            }
            if fetched.elapsed() < Duration::from_secs(60) {
                return None;
            }
        }
        let url = format!(
            "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
            self.tenant.as_deref().unwrap_or("organizations")
        );
        let jwks = tokio::task::spawn_blocking(move || -> Result<Jwks, String> {
            let body = ureq::get(&url)
                .call()
                .map_err(|error| error.to_string())?
                .body_mut()
                .read_to_string()
                .map_err(|error| error.to_string())?;
            serde_json::from_str(&body).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        let jwks = match jwks {
            Ok(jwks) => jwks,
            Err(error) => {
                eprintln!("TeamsFast relay: could not load Microsoft signing keys: {error}");
                return None;
            }
        };
        let keys: HashMap<_, _> = jwks
            .keys
            .into_iter()
            .filter_map(|jwk| {
                let key = DecodingKey::from_rsa_components(&jwk.n?, &jwk.e?).ok()?;
                Some((jwk.kid, key))
            })
            .collect();
        let key = keys.get(kid).cloned();
        if let Ok(mut cache) = self.keys.lock() {
            *cache = Some((Instant::now(), keys));
        }
        key
    }
}
