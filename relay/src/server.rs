use crate::{Change, Changes, EncryptedContent, RegisterClient, Registration, identity::Identity};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

const MAX_CLIENTS: usize = 64;
const MAX_EVENTS: usize = 256;
const CLIENT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
// ponytail: encrypted payloads stay in the bounded in-memory queue (worst case MAX_CLIENTS ×
// MAX_EVENTS × ~64 KB); oversized ones are dropped and the desktop fetches the message instead.
const MAX_CONTENT: usize = 48 * 1024;
const MAX_TOKENS: usize = 16 * 1024;

#[derive(Clone)]
struct Relay {
    public_url: Arc<String>,
    key: Arc<String>,
    identity: Option<Arc<Identity>>,
    clients: Arc<Mutex<HashMap<String, Arc<Mutex<Client>>>>>,
}

struct Client {
    key: String,
    client_state: String,
    events: VecDeque<(u64, Change)>,
    cursor: u64,
    touched: Instant,
    notify: Arc<Notify>,
}

/// `client_id`/`tenant` enable registration with a Microsoft ID token for that Entra app; the
/// operator `key` always works (scripts, probes).
pub fn router(
    public_url: &str,
    key: &str,
    client_id: Option<&str>,
    tenant: Option<&str>,
) -> Result<Router, &'static str> {
    let uri: Uri = public_url.parse().map_err(|_| "Invalid public URL")?;
    let local = matches!(uri.host(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if !(uri.scheme_str() == Some("https") || (local && uri.scheme_str() == Some("http")))
        || uri.host().is_none()
        || uri.query().is_some()
        || uri
            .authority()
            .is_some_and(|authority| authority.as_str().contains('@'))
    {
        return Err("Public URL must use HTTPS (HTTP loopback is allowed for local tests)");
    }
    if !(32..=256).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("TEAMSFAST_RELAY_KEY must contain 32–256 printable ASCII characters");
    }
    let state = Relay {
        public_url: Arc::new(public_url.trim_end_matches('/').into()),
        key: Arc::new(key.into()),
        identity: client_id.map(|id| Arc::new(Identity::new(id, tenant))),
        clients: Arc::new(Mutex::new(HashMap::new())),
    };
    Ok(Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/clients", post(register))
        .route("/v1/clients/{id}", axum::routing::delete(unregister))
        .route("/v1/clients/{id}/events", get(events))
        .route("/graph/{id}", post(webhook))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state))
}

fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("")
}

fn same(a: &str, b: &str) -> bool {
    !b.is_empty() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// The operator key, or the registration's own key.
fn authorized(headers: &HeaderMap, relay: &Relay, client: &Client) -> Result<(), StatusCode> {
    let value = bearer(headers);
    if same(value, &relay.key) || same(value, &client.key) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn random_key() -> Result<String, StatusCode> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn register(
    State(relay): State<Relay>,
    headers: HeaderMap,
    Json(request): Json<RegisterClient>,
) -> Result<Json<Registration>, StatusCode> {
    let token = bearer(&headers);
    if !same(token, &relay.key) {
        match &relay.identity {
            Some(identity) if identity.verify(token).await => {}
            _ => return Err(StatusCode::UNAUTHORIZED),
        }
    }
    if !valid_id(&request.installation_id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut clients = relay
        .clients
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    clients.retain(|_, client| {
        client
            .lock()
            .is_ok_and(|client| client.touched.elapsed() < CLIENT_TTL)
    });
    if !clients.contains_key(&request.installation_id) {
        if clients.len() >= MAX_CLIENTS {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        clients.insert(
            request.installation_id.clone(),
            Arc::new(Mutex::new(Client {
                key: random_key()?,
                client_state: random_key()?,
                events: VecDeque::new(),
                cursor: 0,
                touched: Instant::now(),
                notify: Arc::new(Notify::new()),
            })),
        );
    }
    let mut client = clients
        .get(&request.installation_id)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    client.touched = Instant::now();
    Ok(Json(Registration {
        notification_url: format!("{}/graph/{}", relay.public_url, request.installation_id),
        installation_id: request.installation_id,
        client_state: client.client_state.clone(),
        cursor: client.cursor,
        client_key: client.key.clone(),
    }))
}

async fn unregister(
    State(relay): State<Relay>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut clients = relay
        .clients
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(client) = clients.get(&id) {
        authorized(
            &headers,
            &relay,
            &*client
                .lock()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )?;
        clients.remove(&id);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: u64,
}

fn snapshot(client: &mut Client, after: u64) -> Changes {
    client.touched = Instant::now();
    let reset = after > client.cursor
        || client
            .events
            .front()
            .is_some_and(|(first, _)| after.saturating_add(1) < *first);
    Changes {
        cursor: client.cursor,
        reset,
        changes: if reset {
            Vec::new()
        } else {
            client
                .events
                .iter()
                .filter(|(id, _)| *id > after)
                .map(|(_, event)| event.clone())
                .collect()
        },
    }
}

async fn events(
    State(relay): State<Relay>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<Changes>, StatusCode> {
    let client = relay
        .clients
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    authorized(
        &headers,
        &relay,
        &*client
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    )?;
    let notify = client
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .notify
        .clone();
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    {
        let batch = snapshot(
            &mut *client
                .lock()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            cursor.after,
        );
        if batch.reset || !batch.changes.is_empty() {
            return Ok(Json(batch));
        }
    }
    // Event-driven long-poll: a webhook wakes this request immediately, with a quiet heartbeat otherwise.
    let _ = tokio::time::timeout(Duration::from_secs(25), notified).await;
    let batch = snapshot(
        &mut *client
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        cursor.after,
    );
    Ok(Json(batch))
}

#[derive(Deserialize)]
struct NotificationBatch {
    value: Vec<Notice>,
    /// `null` when Graph could not attach resource data.
    #[serde(default, rename = "validationTokens")]
    validation_tokens: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    client_state: Option<String>,
    #[serde(default)]
    resource: String,
    #[serde(default)]
    change_type: String,
    lifecycle_event: Option<String>,
    encrypted_content: Option<EncryptedContent>,
}

fn resource_change(resource: &str, kind: &str) -> Option<Change> {
    if !matches!(kind, "created" | "updated" | "deleted") || resource.len() > 4096 {
        return None;
    }
    let resource = resource.trim_start_matches('/');
    if let Some(rest) = resource.strip_prefix("chats('") {
        let (chat, suffix) = rest.split_once("')")?;
        if chat.is_empty() {
            return None;
        }
        let message = suffix
            .strip_prefix("/messages('")
            .and_then(|s| s.strip_suffix("')"));
        return Some(Change {
            chat_id: Some(chat.into()),
            message_id: message.map(str::to_owned),
            kind: kind.into(),
            ..Default::default()
        });
    }
    let parts: Vec<_> = resource.split('/').collect();
    if parts.first() == Some(&"chats") && parts.get(1).is_some_and(|id| !id.is_empty()) {
        return Some(Change {
            chat_id: Some(parts[1].into()),
            message_id: if parts.get(2) == Some(&"messages") {
                parts.get(3).map(|s| (*s).into())
            } else {
                None
            },
            kind: kind.into(),
            ..Default::default()
        });
    }
    if resource.starts_with("users/") || resource.starts_with("users('") {
        return Some(Change {
            kind: "chats".into(),
            ..Default::default()
        });
    }
    None
}

async fn webhook(
    State(relay): State<Relay>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let client = match relay
        .clients
        .lock()
        .ok()
        .and_then(|clients| clients.get(&id).cloned())
    {
        Some(client) => client,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    if let Some(token) = query.get("validationToken") {
        if token.len() > 4096 {
            return StatusCode::BAD_REQUEST.into_response();
        }
        return (
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            token.clone(),
        )
            .into_response();
    }
    let batch: NotificationBatch = match serde_json::from_slice(&body) {
        Ok(batch) => batch,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if batch.value.len() > 100 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let tokens = batch.validation_tokens.unwrap_or_default();
    let tokens_fit = tokens.iter().map(String::len).sum::<usize>() <= MAX_TOKENS;
    let Ok(mut client) = client.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    for notice in batch.value {
        if !bool::from(
            notice
                .client_state
                .as_deref()
                .unwrap_or("")
                .as_bytes()
                .ct_eq(client.client_state.as_bytes()),
        ) {
            continue;
        }
        let change = if notice.lifecycle_event.is_some() {
            Some(Change {
                kind: "resync".into(),
                ..Default::default()
            })
        } else {
            resource_change(&notice.resource, &notice.change_type).map(|mut change| {
                // Passed through opaque: the relay cannot decrypt it.
                if let Some(content) = notice.encrypted_content
                    && change.message_id.is_some()
                    && tokens_fit
                    && !tokens.is_empty()
                    && content.data.len()
                        + content.data_key.len()
                        + content.data_signature.len()
                        + content.encryption_certificate_id.len()
                        <= MAX_CONTENT
                {
                    change.content = Some(content);
                    change.tokens = tokens.clone();
                }
                change
            })
        };
        if let Some(change) = change {
            client.cursor = client.cursor.saturating_add(1);
            let cursor = client.cursor;
            client.events.push_back((cursor, change));
            while client.events.len() > MAX_EVENTS {
                client.events.pop_front();
            }
        }
    }
    client.notify.notify_waiters();
    StatusCode::ACCEPTED.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    const KEY: &str = "local-test-key-0123456789abcdef0123456789";

    #[tokio::test]
    async fn authenticated_registration_validation_and_push_wake_the_waiting_client() {
        let app = router("https://relay.example", KEY, None, None).unwrap();
        let id = "a".repeat(64);
        let request = || {
            Request::post("/v1/clients")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {KEY}"))
                .body(Body::from(
                    serde_json::json!({"installation_id": id}).to_string(),
                ))
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let registration: Registration =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        let validation = app
            .clone()
            .oneshot(
                Request::post(format!("/graph/{id}?validationToken=test%20value"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(validation.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(validation.into_body(), 4096).await.unwrap(),
            "test value"
        );
        let waiting = tokio::spawn(
            app.clone().oneshot(
                Request::get(format!("/v1/clients/{id}/events?after=0"))
                    .header("authorization", format!("Bearer {KEY}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
        );
        tokio::task::yield_now().await;
        let body = serde_json::json!({"value": [
            {"clientState": "wrong", "resource": "chats/other/messages/private", "changeType": "created"},
            {"clientState": registration.client_state, "resource": "chats('chat-1')/messages('message-1')", "changeType": "created"}
        ]});
        let response = app
            .clone()
            .oneshot(
                Request::post(format!("/graph/{id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let response = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let changes: Changes =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(changes.changes.len(), 1);
        assert_eq!(changes.changes[0].message_id.as_deref(), Some("message-1"));
        let unauthorized = app
            .oneshot(
                Request::get(format!("/v1/clients/{id}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn bounded_history_reports_gaps_and_accepts_only_known_resource_shapes() {
        let mut client = Client {
            key: String::new(),
            client_state: String::new(),
            events: VecDeque::from([(
                12,
                Change {
                    kind: "chats".into(),
                    ..Default::default()
                },
            )]),
            cursor: 12,
            touched: Instant::now(),
            notify: Arc::new(Notify::new()),
        };
        assert!(snapshot(&mut client, 1).reset);
        assert!(snapshot(&mut client, 99).reset);
        assert!(!snapshot(&mut client, 11).reset);
        assert!(resource_change("https://evil.example/chats/id", "created").is_none());
        assert!(router("http://public.example", KEY, None, None).is_err());
        assert!(router("https://relay.example", "short", None, None).is_err());
    }
}
