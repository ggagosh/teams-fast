use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub client_id: String,
    pub tenant: String,
    pub remember: bool,
    pub relay_url: String,
    pub notifications: bool,
    pub notification_previews: bool,
    pub quiet: bool,
    pub light_theme: bool,
    /// Link previews fetch the linked page directly, which tells that site you opened the chat.
    pub hide_link_previews: bool,
    pub drafts: BTreeMap<String, BTreeMap<String, String>>,
    pub muted: BTreeMap<String, BTreeSet<String>>,
}

impl Settings {
    pub fn load() -> Result<Self, String> {
        let directory = directory()?;
        let path = directory.join("settings.json");
        let mut settings: Self = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)
                .map_err(|error| format!("Could not read saved settings: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::read_to_string(directory.join("app.ron")) {
                    Ok(ron) => {
                        let legacy: BTreeMap<String, String> =
                            ron::from_str(&ron).map_err(|error| {
                                format!("Could not migrate the previous settings: {error}")
                            })?;
                        legacy
                            .get("settings-v1")
                            .map(|json| serde_json::from_str(json))
                            .transpose()
                            .map_err(|error| format!("Could not migrate saved drafts: {error}"))?
                            .unwrap_or_default()
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
                    Err(error) => return Err(format!("Could not read previous settings: {error}")),
                }
            }
            Err(error) => return Err(format!("Could not read saved settings: {error}")),
        };
        // Runtime environment (`just dev` loads `.env`) wins, then saved values, then the IDs
        // baked in at build time (release CI), so a release opens straight to sign-in.
        for (name, target, built) in [
            (
                "TEAMSFAST_CLIENT_ID",
                &mut settings.client_id,
                option_env!("TEAMSFAST_CLIENT_ID"),
            ),
            (
                "TEAMSFAST_TENANT",
                &mut settings.tenant,
                option_env!("TEAMSFAST_TENANT"),
            ),
            (
                "TEAMSFAST_RELAY_URL",
                &mut settings.relay_url,
                option_env!("TEAMSFAST_RELAY_URL"),
            ),
        ] {
            if let Ok(value) = std::env::var(name) {
                *target = value;
            } else if target.is_empty()
                && let Some(value) = built
            {
                *target = value.into();
            }
        }
        if settings.tenant.is_empty() {
            settings.tenant = "organizations".into();
        }
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        write_private("settings.json", &bytes)
    }
}

/// Atomically replaces an owner-only (0600) file in the application data folder.
fn write_private(name: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let directory = directory()?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create settings folder: {error}"))?;
    let pending = directory.join(format!(".{name}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&pending)
        .map_err(|error| format!("Could not save {name}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not save {name}: {error}"))?;
    std::fs::rename(pending, directory.join(name))
        .map_err(|error| format!("Could not replace {name}: {error}"))
}

fn directory() -> Result<std::path::PathBuf, String> {
    let base =
        directories::BaseDirs::new().ok_or("Could not locate the application data folder.")?;
    #[cfg(target_os = "macos")]
    {
        Ok(base.data_dir().join("TeamsFast"))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(base.data_dir().join("TeamsFast").join("data"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(base.data_dir().join("teamsfast"))
    }
}

/// Debug and release builds are signed by different identities, so each keeps its own Keychain
/// items; sharing them makes macOS ask for the password whenever the other build wrote last.
const SERVICE: &str = if cfg!(debug_assertions) {
    "dev.teamsfast.desktop.debug"
} else {
    "dev.teamsfast.desktop"
};

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, account)
        .map_err(|e| format!("Could not open the OS credential store: {e}"))
}

/// Secrets live only in the OS credential store. Debug builds are signed with a stable identity
/// (`scripts/sign_dev.sh`), so rebuilt binaries keep Keychain access without prompts.
pub(crate) fn read_secret(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => legacy_secret(account),
        Err(error) => Err(format!("Could not read the OS credential store: {error}")),
    }
}

pub(crate) fn write_secret(account: &str, secret: &str) -> Result<(), String> {
    entry(account)?
        .set_password(secret)
        .map_err(|e| format!("Could not save to the OS credential store: {e}"))
}

pub(crate) fn delete_secret(account: &str) -> Result<(), String> {
    let _ = legacy_secret(account);
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Could not remove the saved credential: {error}")),
    }
}

// ponytail: moves secrets from the old debug-only `dev-secrets.json` into the Keychain on first
// read, deleting the file once empty. Remove when no old dev installs remain.
fn legacy_secret(account: &str) -> Result<Option<String>, String> {
    let path = directory()?.join("dev-secrets.json");
    let Ok(json) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let mut secrets: BTreeMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
    let Some(value) = secrets.remove(account) else {
        return Ok(None);
    };
    write_secret(account, &value)?;
    if secrets.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        let bytes = serde_json::to_vec_pretty(&secrets).map_err(|error| error.to_string())?;
        write_private("dev-secrets.json", &bytes)?;
    }
    Ok(Some(value))
}

pub(crate) fn auth_key(client: &str, tenant: &str) -> String {
    format!("auth:{tenant}:{client}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_settings_do_not_include_credentials_and_drafts_are_account_scoped() {
        let mut settings = Settings::default();
        settings
            .drafts
            .entry("account-a".into())
            .or_default()
            .insert("chat".into(), "draft".into());
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("access_token"));
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("relay_key"));
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(!restored.drafts.contains_key("account-b"));
    }
}
