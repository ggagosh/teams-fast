use crate::Wake;
use reqwest::{StatusCode, Url, blocking::Client, redirect::Policy};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};
use teamsfast_relay::{Changes, RegisterClient, Registration};

pub(crate) enum PushEvent {
    Ready(Registration),
    Changes(Changes, bool),
    Status(String, bool),
}

pub(crate) struct PushClient {
    pub events: Receiver<PushEvent>,
    stop: Arc<AtomicBool>,
}

pub(crate) fn base_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Enter a valid relay URL.".to_owned())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || local && url.scheme() == "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "The relay must use HTTPS; HTTP is allowed only on loopback for local development."
                .into(),
        );
    }
    Ok(url)
}

fn endpoint(base: &Url, path: &str) -> Result<Url, String> {
    Url::parse(&format!("{}/{}", base.as_str().trim_end_matches('/'), path))
        .map_err(|_| "Invalid relay endpoint.".into())
}

/// The relay's whole bounded queue (256 events, each with at most ~64 KB of encrypted content).
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Relay returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the relay response.".to_owned())?;
    if bytes.len() > MAX_RESPONSE as usize {
        return Err("Relay response was too large.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Relay returned an unreadable response.".into())
}

impl PushClient {
    /// `credential` returns the signed-in user's ID token; the relay accepts it for registration
    /// and hands back a per-registration key for polling.
    pub fn start(
        url: String,
        credential: impl Fn() -> Result<String, String> + Send + 'static,
        wake: Wake,
    ) -> Self {
        let (sender, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        thread::spawn(move || {
            let emit = |event| {
                let _ = sender.send(event);
                let _ = wake.try_send(());
            };
            let run = || -> Result<(), String> {
                let base = base_url(&url)?;
                let client = Client::builder()
                    .user_agent(concat!("TeamsFast/", env!("CARGO_PKG_VERSION")))
                    .redirect(Policy::none())
                    .connect_timeout(Duration::from_secs(8))
                    .timeout(Duration::from_secs(35))
                    .build()
                    .map_err(|_| "Could not initialize the relay connection.".to_owned())?;
                let mut random = [0; 32];
                getrandom::fill(&mut random)
                    .map_err(|_| "Could not create a secure relay client ID.".to_owned())?;
                let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
                let mut registration: Option<Registration> = None;
                let mut cursor = 0;
                let mut recovered = false;
                let mut backoff = 1;
                while !stopped.load(Ordering::Relaxed) {
                    let attempt = || -> Result<(Option<Registration>, Changes), String> {
                        let Some(current) = &registration else {
                            let response = client
                                .post(endpoint(&base, "v1/clients")?)
                                .bearer_auth(credential()?)
                                .json(&RegisterClient {
                                    installation_id: id.clone(),
                                })
                                .send()
                                .map_err(|_| {
                                    "Could not connect to the notification relay.".to_owned()
                                })?;
                            let registered: Registration = decode(response)?;
                            let expected = endpoint(&base, &format!("graph/{id}"))?;
                            if registered.installation_id != id
                                || registered.notification_url != expected.as_str()
                                || registered.client_state.len() < 32
                                || registered.client_key.len() < 32
                            {
                                return Err(
                                    "The relay returned unexpected registration details.".into()
                                );
                            }
                            let batch = Changes {
                                cursor: registered.cursor,
                                reset: true,
                                changes: Vec::new(),
                            };
                            return Ok((Some(registered), batch));
                        };
                        let response = client
                            .get(endpoint(&base, &format!("v1/clients/{id}/events"))?)
                            .bearer_auth(&current.client_key)
                            .query(&[("after", cursor)])
                            .send()
                            .map_err(|_| "Live updates disconnected; reconnecting…".to_owned())?;
                        if matches!(
                            response.status(),
                            StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED
                        ) {
                            return Err("relay-registration-expired".into());
                        }
                        Ok((None, decode(response)?))
                    };
                    match attempt() {
                        Ok((new_registration, batch)) => {
                            if stopped.load(Ordering::Relaxed) {
                                break;
                            }
                            if let Some(value) = new_registration {
                                emit(PushEvent::Ready(value.clone()));
                                registration = Some(value);
                            }
                            cursor = batch.cursor;
                            emit(PushEvent::Status("Relay connected".into(), true));
                            if batch.reset || recovered || !batch.changes.is_empty() {
                                emit(PushEvent::Changes(batch, recovered));
                            }
                            recovered = false;
                            backoff = 1;
                        }
                        Err(error) => {
                            if error == "relay-registration-expired" {
                                registration = None;
                            }
                            emit(PushEvent::Status(
                                if error == "relay-registration-expired" {
                                    "Reconnecting live updates…".into()
                                } else {
                                    error
                                },
                                false,
                            ));
                            recovered = true;
                            for _ in 0..backoff * 10 {
                                if stopped.load(Ordering::Relaxed) {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(100));
                            }
                            backoff = (backoff * 2).min(30);
                        }
                    }
                }
                if let Some(registration) = &registration {
                    let _ = client
                        .delete(endpoint(&base, &format!("v1/clients/{id}"))?)
                        .bearer_auth(&registration.client_key)
                        .send();
                }
                Ok(())
            };
            if let Err(error) = run() {
                emit(PushEvent::Status(error, false));
            }
        });
        Self { events, stop }
    }
}

impl Drop for PushClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_urls_require_tls_without_credentials_or_queries() {
        assert!(base_url("https://relay.example").is_ok());
        assert!(base_url("http://127.0.0.1:8787").is_ok());
        for value in [
            "http://public.example",
            "https://key@relay.example",
            "https://relay.example?key=secret",
            "file:///tmp/x",
        ] {
            assert!(base_url(value).is_err());
        }
    }
}
