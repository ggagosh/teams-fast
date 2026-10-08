use crate::Wake;
use crate::{
    model::{ChatSummary, Message, MessageChange, Person},
    settings,
};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    redirect::Policy,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use teamsfast_relay::Registration;

// Delegated Graph permissions (all consented in the app registration). `openid` adds an ID token,
// which the relay accepts instead of a shared key. `Chat.ReadWrite` covers reading chats and, later,
// read state and edits; `User.ReadBasic.All` covers profile photos and people search.
const SCOPES: &str = "openid offline_access https://graph.microsoft.com/User.Read https://graph.microsoft.com/User.ReadBasic.All https://graph.microsoft.com/Chat.ReadWrite https://graph.microsoft.com/Chat.Create https://graph.microsoft.com/ChatMessage.Send";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Graph ID of the "Name (You)" self chat.
const SELF_CHAT: &str = "48:notes";
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 512 * 1024;

#[derive(Clone)]
pub(crate) struct AccountConfig {
    pub client_id: String,
    pub tenant: String,
    pub remember: bool,
    /// Device-code sign-in instead of the browser redirect (another device, or no localhost redirect).
    pub use_code: bool,
}
impl AccountConfig {
    fn scopes(&self) -> &'static str {
        SCOPES
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Initial,
    Refresh,
    Older,
    Catchup,
    /// Read-only history inspection; does not load the transcript or mark anything read.
    Activity,
}

#[derive(Clone)]
pub(crate) struct Rich {
    pub content: teamsfast_relay::EncryptedContent,
    pub tokens: Vec<String>,
}

/// Background downloads that never block chat traffic.
pub(crate) enum Media {
    Photo(String),
    Image(String),
    Preview(String),
}

pub(crate) enum Command {
    SignIn(AccountConfig),
    Resume(AccountConfig),
    Forget(AccountConfig),
    SignOut,
    Chats {
        next: Option<String>,
        background: bool,
        started_revision: u64,
    },
    Messages {
        chat_id: String,
        next: Option<String>,
        kind: LoadKind,
        started_revision: u64,
    },
    Message {
        chat_id: String,
        message_id: String,
        notify: bool,
        deleted: bool,
        /// Encrypted message from a rich notification; when it verifies, no fetch is needed.
        rich: Option<Rich>,
    },
    Send {
        chat_id: String,
        /// ID of the local placeholder message this send confirms.
        local_id: String,
        text: String,
    },
    Mutate {
        chat_id: String,
        message_id: String,
        operation_id: u64,
        change: MessageChange,
    },
    MarkRead {
        chat_id: String,
        marker: String,
        active: Arc<AtomicBool>,
        deadline: Instant,
    },
    People {
        query: String,
    },
    Create {
        people: Vec<String>,
        topic: String,
    },
    Watch(Option<Registration>),
    Renew,
}
#[derive(Clone)]
pub(crate) enum Operation {
    SignIn,
    Chats,
    Messages(String, LoadKind),
    Message(String),
    /// Chat ID and local placeholder ID.
    Send(String, String),
    Mutate(String, String, u64),
    MarkRead(String, String),
    People(String),
    Create,
    Watch,
}

pub(crate) enum Event {
    DeviceCode {
        user_code: String,
        url: String,
    },
    /// Browser sign-in started; the page can be reopened from the app.
    SignInPage(String),
    Connected {
        user_id: String,
        name: String,
    },
    Chats {
        chats: Vec<ChatSummary>,
        next: Option<String>,
        background: bool,
        started_revision: u64,
    },
    Messages {
        chat_id: String,
        messages: Vec<Message>,
        next: Option<String>,
        kind: LoadKind,
        started_revision: u64,
    },
    MeetingActivity {
        chat_id: String,
        has_messages: bool,
        next: Option<String>,
        started_revision: u64,
    },
    Changed {
        chat_id: String,
        message: Message,
        notify: bool,
    },
    Deleted {
        chat_id: String,
        message_id: String,
    },
    Sent {
        chat_id: String,
        local_id: String,
        message: Message,
    },
    Mutated {
        chat_id: String,
        message_id: String,
        operation_id: u64,
        message: Message,
    },
    Read {
        chat_id: String,
        marker: String,
    },
    People {
        query: String,
        people: Vec<Person>,
    },
    Created(ChatSummary),
    Watching {
        active: usize,
        total: usize,
    },
    /// `None` when the person has no photo or it could not be loaded; initials remain.
    Photo {
        user_id: String,
        photo: Option<(String, Vec<u8>)>,
    },
    /// A message image by URL, decoded at display size with its byte size; `None` when it could
    /// not be loaded.
    Image {
        url: String,
        image: Option<(Arc<gpui_kit::RenderImage>, usize)>,
    },
    Preview {
        url: String,
        preview: Option<crate::model::Preview>,
    },
    Warning(String),
    Failed {
        operation: Operation,
        error: Failure,
    },
}

pub(crate) struct Worker {
    commands: SyncSender<(u64, Command, Instant)>,
    sends: SyncSender<(u64, Command, Instant)>,
    updates: SyncSender<(u64, Command, Instant)>,
    watches: mpsc::Sender<(u64, WatchCommand)>,
    media: mpsc::Sender<(u64, Media)>,
    active_session: SharedSession,
    pub events: Receiver<(u64, Event)>,
    generation: Arc<AtomicU64>,
}
impl Worker {
    pub fn new(wake: Wake) -> Self {
        let (commands, requests) = mpsc::sync_channel(32);
        let (events, responses) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let active_session: SharedSession = Arc::new(Mutex::new(None));
        let (sends, send_requests) = mpsc::sync_channel(16);
        let (updates, update_requests) = mpsc::sync_channel(32);
        let (watches, watch_requests) = mpsc::channel();
        let (media, media_requests) = mpsc::channel();
        spawn_media_lane(
            media_requests,
            active_session.clone(),
            events.clone(),
            wake.clone(),
            generation.clone(),
        );
        // Sends keep Microsoft's 1 request/second per-chat budget; incoming reads need no gap
        // (throttling still applies through Retry-After).
        for (requests, spacing) in [
            (send_requests, Duration::from_secs(1)),
            (update_requests, Duration::ZERO),
        ] {
            spawn_request_lane(
                requests,
                spacing,
                active_session.clone(),
                events.clone(),
                wake.clone(),
                generation.clone(),
            );
        }
        spawn_watch_lane(
            watch_requests,
            events.clone(),
            wake.clone(),
            generation.clone(),
        );
        let shared_session = active_session.clone();
        let watch_commands = watches.clone();
        let current = Arc::clone(&generation);
        let credential_lock = Arc::new(Mutex::new(()));
        thread::spawn(move || {
            let client = Client::builder()
                .https_only(true)
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(20))
                .user_agent(concat!("TeamsFast/", env!("CARGO_PKG_VERSION")))
                .build();
            let mut session: Option<Session> = None;
            let mut cooldown: Option<Instant> = None;
            while let Ok((epoch, command, queued_at)) = requests.recv() {
                if epoch != current.load(Ordering::Relaxed) {
                    continue;
                }
                let started = Instant::now();
                let emit = |event| {
                    let _ = events.send((epoch, event));
                    let _ = wake.try_send(());
                };
                if matches!(command, Command::SignOut) {
                    session = None;
                    cooldown = None;
                    let _ = watch_commands.send((epoch, WatchCommand::Reset));
                    continue;
                }
                if let Command::Forget(config) = command {
                    let Ok(_guard) = credential_lock.lock() else {
                        emit(Event::Warning(
                            "Could not lock the credential store.".into(),
                        ));
                        continue;
                    };
                    if let Err(error) = settings::delete_secret(&settings::auth_key(
                        &config.client_id,
                        &config.tenant,
                    )) {
                        emit(Event::Warning(error));
                    }
                    continue;
                }
                let operation = match &command {
                    Command::SignIn(_) | Command::Resume(_) => Operation::SignIn,
                    Command::Chats { .. } => Operation::Chats,
                    Command::Messages { chat_id, kind, .. } => {
                        Operation::Messages(chat_id.clone(), *kind)
                    }
                    Command::Message { chat_id, .. } => Operation::Message(chat_id.clone()),
                    Command::Send {
                        chat_id, local_id, ..
                    } => Operation::Send(chat_id.clone(), local_id.clone()),
                    Command::People { query } => Operation::People(query.clone()),
                    Command::Create { .. } => Operation::Create,
                    _ => unreachable!(),
                };
                let result = (|| {
                    let client = client
                        .as_ref()
                        .map_err(|_| Failure::new("Could not initialize HTTPS."))?;
                    if let Some(until) = cooldown.filter(|until| *until > Instant::now()) {
                        return Err(Failure::throttled(
                            until.saturating_duration_since(Instant::now()),
                        ));
                    }
                    if matches!(&command, Command::SignIn(_) | Command::Resume(_)) {
                        session = None;
                        let _ = watch_commands.send((epoch, WatchCommand::Reset));
                        let mut signed_in = match command {
                            Command::SignIn(config) if config.use_code => {
                                sign_in(client, config, epoch, &current, &emit)?
                            }
                            Command::SignIn(config) => {
                                sign_in_browser(client, config, epoch, &current, &emit)?
                            }
                            Command::Resume(config) => Session::resume(
                                client,
                                config,
                                epoch,
                                current.clone(),
                                credential_lock.clone(),
                            )?,
                            _ => unreachable!(),
                        };
                        if epoch != current.load(Ordering::Relaxed) {
                            return Err(Failure::new("Sign-in canceled."));
                        }
                        signed_in.owner = Some((epoch, current.clone()));
                        signed_in.credential_lock = credential_lock.clone();
                        let user: User = signed_in.get(client, graph_url(&["me"])?)?;
                        signed_in.user_id = user.id.clone();
                        if let Err(error) = signed_in.save() {
                            emit(Event::Warning(error));
                        }
                        if let Ok(mut shared) = shared_session.lock() {
                            *shared = Some((epoch, signed_in.clone()));
                        }
                        let _ =
                            watch_commands.send((epoch, WatchCommand::Session(signed_in.clone())));
                        session = Some(signed_in);
                        return Ok(Event::Connected {
                            user_id: user.id,
                            name: user.display_name,
                        });
                    }
                    let session = session
                        .as_mut()
                        .ok_or_else(|| Failure::new("Sign in to load Teams chats."))?;
                    match command {
                        Command::Chats {
                            next,
                            background,
                            started_revision,
                        } => {
                            let url = match next {
                                Some(link) => checked_graph_url(&link)?,
                                None => {
                                    let mut url = graph_url(&["me", "chats"])?;
                                    url.query_pairs_mut()
                                        .append_pair("$top", "50")
                                        .append_pair("$expand", "members,lastMessagePreview")
                                        .append_pair(
                                            "$orderby",
                                            "lastMessagePreview/createdDateTime desc",
                                        );
                                    url
                                }
                            };
                            let page: Page<GraphChat> = session.get(client, url)?;
                            let mut chats: Vec<_> = page
                                .value
                                .into_iter()
                                .map(|chat| chat.summary(&session.user_id))
                                .collect();
                            let _ = watch_commands.send((
                                epoch,
                                WatchCommand::Resources(
                                    chats
                                        .iter()
                                        .map(|chat| format!("chats/{}/messages", chat.id))
                                        .collect(),
                                ),
                            ));
                            // The self chat ("Name (You)") is undocumented and absent from the
                            // listing; Graph rejects `chats/48:notes` itself ("not a ChatThread")
                            // but serves its messages. Build its entry from the newest message once
                            // the last page is in. Not subscribed for live updates.
                            if page.next.is_none() && !background {
                                let mut url = graph_url(&["chats", SELF_CHAT, "messages"])?;
                                url.query_pairs_mut().append_pair("$top", "1");
                                if let Ok(latest) = session.get::<Page<GraphMessage>>(client, url) {
                                    let latest = latest
                                        .value
                                        .into_iter()
                                        .next()
                                        .map(|message| message.display(&session.user_id));
                                    chats.push(ChatSummary {
                                        id: SELF_CHAT.into(),
                                        title: "Notes (You)".into(),
                                        members: 1,
                                        is_self: true,
                                        preview_mine: true,
                                        preview: latest
                                            .as_ref()
                                            .map(|m| m.text.replace('\n', " "))
                                            .unwrap_or_default(),
                                        preview_id: latest.as_ref().map(|m| m.id.clone()),
                                        updated_at: latest
                                            .map(|m| m.created_at)
                                            .unwrap_or_default(),
                                        ..Default::default()
                                    });
                                }
                            }
                            Ok(Event::Chats {
                                chats,
                                next: page.next,
                                background,
                                started_revision,
                            })
                        }
                        Command::Messages {
                            chat_id,
                            next,
                            kind,
                            started_revision,
                        } => {
                            let url = match next {
                                Some(link) => checked_graph_url(&link)?,
                                None => {
                                    let mut url = graph_url(&["chats", &chat_id, "messages"])?;
                                    url.query_pairs_mut()
                                        .append_pair("$top", "50")
                                        .append_pair("$orderby", "createdDateTime desc");
                                    url
                                }
                            };
                            let page: Page<GraphMessage> = session.get(client, url)?;
                            if kind == LoadKind::Activity {
                                return Ok(Event::MeetingActivity {
                                    chat_id,
                                    // Event details also identify system activity when Graph masks
                                    // an evolvable enum as unknownFutureValue.
                                    has_messages: page
                                        .value
                                        .iter()
                                        .any(|message| !message.is_system_event()),
                                    next: page.next,
                                    started_revision,
                                });
                            }
                            Ok(Event::Messages {
                                chat_id,
                                messages: page
                                    .value
                                    .into_iter()
                                    .map(|message| message.display(&session.user_id))
                                    .collect(),
                                next: page.next,
                                kind,
                                started_revision,
                            })
                        }
                        Command::People { query } => {
                            let query = query.trim().chars().take(100).collect::<String>();
                            let literal = query.replace('\'', "''");
                            let mut url = graph_url(&["users"])?;
                            url.query_pairs_mut().append_pair("$top","20").append_pair("$select","id,displayName,mail,userPrincipalName")
                                .append_pair("$filter",&format!("startsWith(displayName,'{literal}') or startsWith(mail,'{literal}') or startsWith(userPrincipalName,'{literal}')"));
                            let page: Page<Person> = session.get(client, url)?;
                            Ok(Event::People {
                                query,
                                people: page
                                    .value
                                    .into_iter()
                                    .filter(|person| person.id != session.user_id)
                                    .collect(),
                            })
                        }
                        Command::Create { mut people, topic } => {
                            people.retain(|id| id != &session.user_id);
                            people.sort();
                            people.dedup();
                            if people.is_empty()
                                || people.len() > 20
                                || people.iter().any(|id| !valid_uuid(id))
                            {
                                return Err(Failure::new(
                                    "Select between 1 and 20 people from the directory.",
                                ));
                            }
                            let group = people.len() > 1 || !topic.trim().is_empty();
                            people.push(session.user_id.clone());
                            let members:Vec<_>=people.iter().map(|id|serde_json::json!({"@odata.type":"#microsoft.graph.aadUserConversationMember","roles":["owner"],"user@odata.bind":format!("https://graph.microsoft.com/v1.0/users('{id}')")})).collect();
                            let mut body = serde_json::json!({"chatType":if group{"group"}else{"oneOnOne"},"members":members});
                            if group && !topic.trim().is_empty() {
                                body["topic"] =
                                    topic.trim().chars().take(200).collect::<String>().into();
                            }
                            session.refresh(client)?;
                            let chat: GraphChat = decode(
                                client
                                    .post(graph_url(&["chats"])?)
                                    .bearer_auth(session.access_token(client)?)
                                    .json(&body)
                                    .send()
                                    .map_err(http_failure)?,
                            )?;
                            let _ = watch_commands.send((
                                epoch,
                                WatchCommand::Resources(vec![format!(
                                    "chats/{}/messages",
                                    chat.id
                                )]),
                            ));
                            Ok(Event::Created(chat.summary(&session.user_id)))
                        }
                        _ => unreachable!(),
                    }
                })();
                if epoch != current.load(Ordering::Relaxed) {
                    session = None;
                    let _ = watch_commands.send((epoch, WatchCommand::Reset));
                    continue;
                }
                trace_latency("read", queued_at, started);
                match result {
                    Ok(event) => emit(event),
                    Err(error) => {
                        if let Some(delay) = error.retry_after {
                            cooldown = Instant::now().checked_add(delay);
                        }
                        emit(Event::Failed { operation, error });
                    }
                }
            }
        });
        Self {
            commands,
            sends,
            updates,
            watches,
            media,
            active_session,
            events: responses,
            generation,
        }
    }
    pub fn epoch(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
    pub fn media(&self, request: Media) {
        let _ = self.media.send((self.epoch(), request));
    }
    pub fn reset(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut session) = self.active_session.lock() {
            *session = None;
        }
        let _ = self.watches.send((self.epoch(), WatchCommand::Reset));
        let _ = self.send(Command::SignOut);
    }
    pub fn send(&self, command: Command) -> Result<(), String> {
        let epoch = self.epoch();
        match command {
            Command::Watch(registration) => self
                .watches
                .send((epoch, WatchCommand::Configure(registration)))
                .map_err(|_| "Live-update worker is unavailable.".into()),
            Command::Renew => self
                .watches
                .send((epoch, WatchCommand::Renew))
                .map_err(|_| "Live-update worker is unavailable.".into()),
            command => {
                let queue = match command {
                    Command::Send { .. } | Command::Mutate { .. } => &self.sends,
                    Command::Message { .. } | Command::MarkRead { .. } => &self.updates,
                    _ => &self.commands,
                };
                queue
                    .try_send((epoch, command, Instant::now()))
                    .map_err(|_| "The network worker is busy or unavailable. Try again.".into())
            }
        }
    }
}
impl Worker {
    /// A fresh ID token for relay registration, refreshing the session if needed. Blocking: call
    /// it off the UI thread (the relay thread does).
    pub fn relay_credential(&self) -> impl Fn() -> Result<String, String> + Send + 'static {
        let shared = self.active_session.clone();
        let generation = self.generation.clone();
        move || {
            let epoch = generation.load(Ordering::Relaxed);
            let mut session = shared
                .lock()
                .map_err(|_| "Could not read sign-in state.".to_owned())?
                .as_ref()
                .filter(|(active, _)| *active == epoch)
                .map(|(_, session)| session.clone())
                .ok_or_else(|| "Sign in to enable live updates.".to_owned())?;
            let client = network_client().map_err(|error| error.message)?;
            session.refresh(&client).map_err(|error| error.message)?;
            session
                .auth
                .lock()
                .map_err(|_| "Could not read sign-in state.".to_owned())?
                .token
                .id_token
                .clone()
                .ok_or_else(|| "Sign in again to enable live updates.".to_owned())
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

enum WatchCommand {
    Session(Session),
    Configure(Option<Registration>),
    Resources(Vec<String>),
    Renew,
    Reset,
}

fn network_client() -> Result<Client, Failure> {
    Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("TeamsFast/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| Failure::new("Could not initialize HTTPS."))
}

pub(crate) fn tracing() -> bool {
    std::env::var("TEAMSFAST_TRACE_LATENCY").as_deref() == Ok("1")
}

fn trace_latency(kind: &str, queued_at: Instant, started: Instant) {
    if std::env::var("TEAMSFAST_TRACE_LATENCY").as_deref() == Ok("1") {
        eprintln!(
            "TeamsFast {kind}: queue={}ms work={}ms",
            started.duration_since(queued_at).as_millis(),
            started.elapsed().as_millis()
        );
    }
}

fn spawn_request_lane(
    requests: Receiver<(u64, Command, Instant)>,
    spacing: Duration,
    active_session: SharedSession,
    events: mpsc::Sender<(u64, Event)>,
    wake: Wake,
    generation: Arc<AtomicU64>,
) {
    thread::spawn(move || {
        let client = network_client();
        let mut ready_at = Instant::now();
        while let Ok((epoch, command, queued_at)) = requests.recv() {
            if generation.load(Ordering::Relaxed) != epoch {
                continue;
            }
            while Instant::now() < ready_at {
                if generation.load(Ordering::Relaxed) != epoch {
                    break;
                }
                thread::sleep(
                    ready_at
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                );
            }
            if generation.load(Ordering::Relaxed) != epoch {
                continue;
            }
            let started = Instant::now();
            let (operation, kind) = match &command {
                Command::Send {
                    chat_id, local_id, ..
                } => (Operation::Send(chat_id.clone(), local_id.clone()), "send"),
                Command::Mutate {
                    chat_id,
                    message_id,
                    operation_id,
                    ..
                } => (
                    Operation::Mutate(chat_id.clone(), message_id.clone(), *operation_id),
                    "mutate",
                ),
                Command::MarkRead {
                    chat_id, marker, ..
                } => (
                    Operation::MarkRead(chat_id.clone(), marker.clone()),
                    "read-marker",
                ),
                Command::Message { chat_id, .. } => {
                    (Operation::Message(chat_id.clone()), "incoming")
                }
                _ => continue,
            };
            let result = (|| {
                let client = client
                    .as_ref()
                    .map_err(|_| Failure::new("Could not initialize HTTPS."))?;
                let mut session = active_session
                    .lock()
                    .map_err(|_| Failure::new("Could not read sign-in state."))?
                    .as_ref()
                    .filter(|(active, _)| *active == epoch)
                    .map(|(_, session)| session.clone())
                    .ok_or_else(|| Failure::new("Sign in to use Teams chats."))?;
                match command {
                    Command::Send {
                        chat_id,
                        local_id,
                        text,
                    } => {
                        if text.trim().is_empty() {
                            return Err(Failure::new("Write a message before sending."));
                        }
                        let token = session.access_token(client)?;
                        if generation.load(Ordering::Relaxed) != epoch {
                            return Err(Failure::new("Session canceled."));
                        }
                        // A timeout can still mean delivery. This POST is never retried automatically.
                        let message: GraphMessage = decode(client.post(graph_url(&["chats",&chat_id,"messages"])?)
                            .bearer_auth(token).json(&serde_json::json!({"body":{"contentType":"text","content":text}}))
                            .send().map_err(http_failure)?)?;
                        Ok(Event::Sent {
                            chat_id,
                            local_id,
                            message: message.display(&session.user_id),
                        })
                    }
                    Command::Mutate {
                        chat_id,
                        message_id,
                        operation_id,
                        change,
                    } => {
                        let token = session.access_token(client)?;
                        if generation.load(Ordering::Relaxed) != epoch {
                            return Err(Failure::new("Session canceled."));
                        }
                        let url = graph_url(&["chats", &chat_id, "messages", &message_id])?;
                        let request = match change {
                            MessageChange::Reaction { kind, set, .. } => client
                                .post(graph_url(&[
                                    "chats",
                                    &chat_id,
                                    "messages",
                                    &message_id,
                                    if set { "setReaction" } else { "unsetReaction" },
                                ])?)
                                .json(&serde_json::json!({"reactionType":kind})),
                            MessageChange::Edit(text) => {
                                if text.trim().is_empty() {
                                    return Err(Failure::new("A message cannot be empty."));
                                }
                                client.patch(url.clone()).json(&serde_json::json!({"body":{"contentType":"text","content":text}}))
                            }
                            MessageChange::Delete => client.post(graph_url(&[
                                "users",
                                &session.user_id,
                                "chats",
                                &chat_id,
                                "messages",
                                &message_id,
                                "softDelete",
                            ])?),
                        };
                        let response = request.bearer_auth(token).send().map_err(http_failure)?;
                        if !response.status().is_success() {
                            decode::<serde_json::Value>(response)?;
                        }
                        // A successful write followed by a failed GET is NOT a rejected write.
                        let message: GraphMessage = session.get(client, url).map_err(|mut e| {
                            e.uncertain = true;
                            e
                        })?;
                        Ok(Event::Mutated {
                            chat_id,
                            message_id,
                            operation_id,
                            message: message.display(&session.user_id),
                        })
                    }
                    Command::MarkRead {
                        chat_id,
                        marker,
                        active,
                        deadline,
                    } => {
                        use base64::Engine as _;
                        let token = session.access_token(client)?;
                        let tenant = {
                            let auth = session
                                .auth
                                .lock()
                                .map_err(|_| Failure::new("Could not read sign-in state."))?;
                            let payload = auth
                                .token
                                .id_token
                                .as_deref()
                                .and_then(|t| t.split('.').nth(1))
                                .and_then(|p| {
                                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                                        .decode(p)
                                        .ok()
                                })
                                .and_then(|p| serde_json::from_slice::<serde_json::Value>(&p).ok());
                            payload
                                .and_then(|p| p.get("tid")?.as_str().map(str::to_owned))
                                .filter(|id| valid_uuid(id))
                                .ok_or_else(|| {
                                    Failure::new("Sign in again to synchronize read state.")
                                })?
                        };
                        // Never replay a stale read intent after scrolling away, switching chats,
                        // a long queue wait, or slow token refresh. Graph accepts no cutoff timestamp.
                        if generation.load(Ordering::Relaxed) != epoch
                            || !active.load(Ordering::Relaxed)
                            || Instant::now() > deadline
                        {
                            return Err(Failure::new(
                                "Read sync deferred; reopen or refresh the conversation to try again.",
                            ));
                        }
                        let response = client.post(graph_url(&["chats", &chat_id, "markChatReadForUser"])?)
                            .bearer_auth(token).json(&serde_json::json!({"user":{"id":session.user_id,"tenantId":tenant}}))
                            .send().map_err(http_failure)?;
                        if !response.status().is_success() {
                            decode::<serde_json::Value>(response)?;
                        }
                        Ok(Event::Read { chat_id, marker })
                    }
                    Command::Message {
                        chat_id,
                        message_id,
                        notify,
                        deleted,
                        rich,
                    } => {
                        if deleted {
                            return Ok(Event::Deleted {
                                chat_id,
                                message_id,
                            });
                        }
                        if let Some(rich) = rich {
                            let reason =
                                match rich_message(client, &session.config.client_id, &rich) {
                                    Ok(message) if message.id == message_id => {
                                        return Ok(Event::Changed {
                                            chat_id,
                                            message: message.display(&session.user_id),
                                            notify,
                                        });
                                    }
                                    Ok(_) => "message ID mismatch".into(),
                                    Err(reason) => reason,
                                };
                            // Reasons are fixed strings or Graph/network errors; never content.
                            if tracing() {
                                eprintln!("TeamsFast rich: fetching instead ({reason})");
                            }
                        }
                        let message: GraphMessage = session.get(
                            client,
                            graph_url(&["chats", &chat_id, "messages", &message_id])?,
                        )?;
                        Ok(Event::Changed {
                            chat_id,
                            message: message.display(&session.user_id),
                            notify,
                        })
                    }
                    _ => unreachable!(),
                }
            })();
            // Respect the per-user/per-conversation request budget without blocking other work.
            ready_at = started + spacing;
            trace_latency(kind, queued_at, started);
            if generation.load(Ordering::Relaxed) != epoch {
                continue;
            }
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    if let Some(delay) = error.retry_after {
                        ready_at = Instant::now() + delay;
                    }
                    Event::Failed { operation, error }
                }
            };
            let _ = events.send((epoch, event));
            let _ = wake.try_send(());
        }
    });
}

/// Photos, message images, and link previews use their own small pool so a chat full of
/// media never delays sends or reads. The Graph token is only sent to graph.microsoft.com.
fn spawn_media_lane(
    requests: Receiver<(u64, Media)>,
    active_session: SharedSession,
    events: mpsc::Sender<(u64, Event)>,
    wake: Wake,
    generation: Arc<AtomicU64>,
) {
    let requests = Arc::new(Mutex::new(requests));
    for _ in 0..3 {
        let requests = requests.clone();
        let active_session = active_session.clone();
        let events = events.clone();
        let wake = wake.clone();
        let generation = generation.clone();
        thread::spawn(move || {
            let (Ok(graph), Ok(web)) = (network_client(), web_client()) else {
                return;
            };
            loop {
                let Ok((epoch, request)) = requests
                    .lock()
                    .map_err(|_| ())
                    .and_then(|r| r.recv().map_err(|_| ()))
                else {
                    return;
                };
                if generation.load(Ordering::Relaxed) != epoch {
                    continue;
                }
                let session = || {
                    active_session.lock().ok().and_then(|shared| {
                        shared
                            .as_ref()
                            .filter(|(active, _)| *active == epoch)
                            .map(|(_, session)| session.clone())
                    })
                };
                // ponytail: any failure (missing, forbidden, throttled, too large) shows nothing; no retry.
                let event = match request {
                    Media::Photo(user_id) => Event::Photo {
                        photo: graph_url(&["users", &user_id, "photos", "48x48", "$value"])
                            .ok()
                            .and_then(|url| fetch_graph_image(&graph, session(), url)),
                        user_id,
                    },
                    Media::Image(url) => Event::Image {
                        image: match checked_graph_url(&url) {
                            Ok(graph_url) => fetch_graph_image(&graph, session(), graph_url),
                            Err(_) => web.get(&url).send().ok().and_then(read_image),
                        }
                        .and_then(|(mime, bytes)| crate::thumbnail::decode(&mime, &bytes)),
                        url,
                    },
                    Media::Preview(url) => Event::Preview {
                        preview: fetch_preview(&web, &url),
                        url,
                    },
                };
                if generation.load(Ordering::Relaxed) == epoch {
                    let _ = events.send((epoch, event));
                    let _ = wake.try_send(());
                }
            }
        });
    }
}

/// Client for public web pages and images: follows redirects, never carries credentials.
fn web_client() -> Result<Client, Failure> {
    Client::builder()
        .https_only(true)
        .redirect(Policy::limited(5))
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(10))
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; TeamsFast/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .build()
        .map_err(|_| Failure::new("Could not initialize HTTPS."))
}

fn fetch_graph_image(
    client: &Client,
    session: Option<Session>,
    url: Url,
) -> Option<(String, Vec<u8>)> {
    let token = session?.access_token(client).ok()?;
    read_image(client.get(url).bearer_auth(token).send().ok()?)
}

fn read_limited(response: Response, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    response
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn content_type(response: &Response) -> Option<String> {
    Some(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase(),
    )
}

fn read_image(response: Response) -> Option<(String, Vec<u8>)> {
    let mime = content_type(&response).filter(|mime| mime.starts_with("image/"))?;
    if !response.status().is_success() {
        return None;
    }
    Some((mime, read_limited(response, MAX_IMAGE_BYTES)?))
}

fn fetch_preview(client: &Client, url: &str) -> Option<crate::model::Preview> {
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() || content_type(&response)? != "text/html" {
        return None;
    }
    let final_url = response.url().clone();
    // Metadata lives in <head>; a truncated page is fine, so read at most the limit.
    let mut bytes = Vec::new();
    response
        .take(MAX_PAGE_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    crate::html::page_preview(&String::from_utf8_lossy(&bytes), &final_url)
}

fn spawn_watch_lane(
    requests: Receiver<(u64, WatchCommand)>,
    events: mpsc::Sender<(u64, Event)>,
    wake: Wake,
    generation: Arc<AtomicU64>,
) {
    thread::spawn(move || {
        let client = network_client();
        let mut session: Option<Session> = None;
        let mut watch = Watch::default();
        let mut active_epoch = 0;
        loop {
            match requests.recv_timeout(
                watch
                    .retry_at
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1)),
            ) {
                Ok((epoch, command)) => {
                    if epoch != generation.load(Ordering::Relaxed) {
                        continue;
                    }
                    active_epoch = epoch;
                    match command {
                        WatchCommand::Session(value) => {
                            watch = Watch::default();
                            // One subscription covers every chat (delegated Chat.Read); the
                            // per-chat resources added later are only a fallback.
                            watch.all_messages =
                                format!("users/{}/chats/getAllMessages", value.user_id);
                            watch.add_resource(watch.all_messages.clone());
                            watch.add_resource(format!("users/{}/chats", value.user_id));
                            session = Some(value);
                        }
                        WatchCommand::Reset => {
                            session = None;
                            watch = Watch::default();
                        }
                        WatchCommand::Configure(registration) => watch.configure(registration),
                        WatchCommand::Resources(resources) => {
                            for resource in resources {
                                watch.add_resource(resource);
                            }
                        }
                        WatchCommand::Renew => {
                            for lease in watch.leases.values_mut() {
                                lease.renew_at = Instant::now();
                            }
                            watch.retry_at = Instant::now();
                        }
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if active_epoch != generation.load(Ordering::Relaxed)
                || watch.registration.is_none()
                || session.is_none()
            {
                watch.retry_at = Instant::now() + Duration::from_secs(1);
                continue;
            }
            if Instant::now() < watch.retry_at {
                continue;
            }
            let started = Instant::now();
            let phase = if watch.discover {
                "Loading subscriptions"
            } else if !watch.cleanup.is_empty() {
                "Replacing previous subscriptions"
            } else {
                "Enabling live chat updates"
            };
            let result = match (&client, &mut session) {
                (Ok(client), Some(session)) => watch.tick(client, session),
                _ => Err(Failure::new("Could not initialize HTTPS.")),
            };
            if active_epoch != generation.load(Ordering::Relaxed) {
                continue;
            }
            watch.retry_at = started + Duration::from_secs(1);
            let event = match result {
                Ok(true) => {
                    let (active, total) = watch.coverage();
                    Some(Event::Watching { active, total })
                }
                Ok(false) => None,
                Err(mut error) => {
                    let delay = error.retry_after.unwrap_or(Duration::from_secs(30));
                    if let Some(resource) = watch.attempted.take() {
                        watch.failed_until.insert(resource, Instant::now() + delay);
                        if error.retry_after.is_some() {
                            watch.retry_at = Instant::now() + delay;
                        }
                    } else {
                        watch.retry_at = Instant::now() + delay;
                    }
                    error.message = format!("{phase}: {}", error.message);
                    Some(Event::Failed {
                        operation: Operation::Watch,
                        error,
                    })
                }
            };
            if let Some(event) = event {
                let _ = events.send((active_epoch, event));
                let _ = wake.try_send(());
            }
        }
    });
}

#[derive(Clone)]
struct Session {
    config: AccountConfig,
    auth: Arc<Mutex<Auth>>,
    user_id: String,
    owner: Option<(u64, Arc<AtomicU64>)>,
    credential_lock: Arc<Mutex<()>>,
}
struct Auth {
    token: Token,
    expires_at: Instant,
}
type SharedSession = Arc<Mutex<Option<(u64, Session)>>>;
#[derive(Serialize, Deserialize)]
struct SavedLogin {
    refresh_token: String,
}
impl Session {
    fn from_token(config: AccountConfig, token: Token) -> Self {
        Self {
            config,
            user_id: String::new(),
            owner: None,
            credential_lock: Arc::new(Mutex::new(())),
            auth: Arc::new(Mutex::new(Auth {
                expires_at: expiry(token.expires_in),
                token,
            })),
        }
    }
    fn resume(
        client: &Client,
        mut config: AccountConfig,
        epoch: u64,
        generation: Arc<AtomicU64>,
        credential_lock: Arc<Mutex<()>>,
    ) -> Result<Self, Failure> {
        validate_registration(&config.client_id, &config.tenant)?;
        let saved = settings::read_secret(&settings::auth_key(&config.client_id, &config.tenant))
            .map_err(|s| Failure::new(&s))?
            .ok_or_else(|| Failure::new("No saved sign-in. Connect your account to continue."))?;
        let saved: SavedLogin = serde_json::from_str(&saved)
            .map_err(|_| Failure::new("The saved sign-in is unreadable. Sign in again."))?;
        config.remember = true;
        let mut session = Self::from_token(
            config,
            Token {
                access_token: String::new(),
                refresh_token: Some(saved.refresh_token),
                expires_in: 0,
                id_token: None,
            },
        );
        session.owner = Some((epoch, generation));
        session.credential_lock = credential_lock;
        session.refresh(client)?;
        Ok(session)
    }
    fn save(&self) -> Result<(), String> {
        if !self.config.remember {
            return Ok(());
        }
        let Some((epoch, generation)) = &self.owner else {
            return Ok(());
        };
        let saved = {
            let auth = self
                .auth
                .lock()
                .map_err(|_| "Could not read the sign-in state.")?;
            let Some(refresh_token) = &auth.token.refresh_token else {
                return Ok(());
            };
            serde_json::to_string(&SavedLogin {
                refresh_token: refresh_token.clone(),
            })
            .map_err(|_| "Could not encode the saved sign-in.".to_owned())?
        };
        // Save and forget share this gate. Revoked generations cannot recreate a credential.
        let _guard = self
            .credential_lock
            .lock()
            .map_err(|_| "Could not lock the credential store.")?;
        if generation.load(Ordering::Relaxed) != *epoch {
            return Ok(());
        }
        settings::write_secret(
            &settings::auth_key(&self.config.client_id, &self.config.tenant),
            &saved,
        )
    }
    fn refresh(&mut self, client: &Client) -> Result<(), Failure> {
        {
            // Only token refresh holds this lock. Message/history/subscription HTTP runs independently.
            let mut auth = self
                .auth
                .lock()
                .map_err(|_| Failure::new("Could not read the sign-in state."))?;
            if Instant::now() < auth.expires_at {
                return Ok(());
            }
            let refresh = auth
                .token
                .refresh_token
                .as_ref()
                .ok_or_else(|| Failure::new("Your session expired. Sign in again."))?;
            let response = client
                .post(oauth_url(&self.config.tenant, "token")?)
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("client_id", self.config.client_id.as_str()),
                    ("refresh_token", refresh.as_str()),
                    ("scope", self.config.scopes()),
                ])
                .send()
                .map_err(http_failure)?;
            let mut token: Token = decode(response)?;
            if token.refresh_token.is_none() {
                token.refresh_token = auth.token.refresh_token.take();
            }
            if token.id_token.is_none() {
                token.id_token = auth.token.id_token.take();
            }
            auth.expires_at = expiry(token.expires_in);
            auth.token = token;
        }
        self.save().map_err(|s| Failure::new(&s))
    }
    fn access_token(&mut self, client: &Client) -> Result<String, Failure> {
        self.refresh(client)?;
        Ok(self
            .auth
            .lock()
            .map_err(|_| Failure::new("Could not read the sign-in state."))?
            .token
            .access_token
            .clone())
    }
    fn get<T: DeserializeOwned>(&mut self, client: &Client, url: Url) -> Result<T, Failure> {
        decode(
            client
                .get(url)
                .header("Prefer", "include-unknown-enum-members")
                .bearer_auth(self.access_token(client)?)
                .send()
                .map_err(http_failure)?,
        )
    }
}

/// Verifies every validation token, then decrypts the message carried by a rich notification.
fn rich_message(client: &Client, client_id: &str, rich: &Rich) -> Result<GraphMessage, String> {
    let keys = crate::rich::keys().ok_or("no notification key")?;
    if rich.tokens.is_empty() {
        return Err("no validation tokens".into());
    }
    for token in &rich.tokens {
        let kid = crate::rich::token_kid(token).ok_or("malformed token")?;
        let key = signing_key(client, &kid).map_err(|error| error.message)?;
        crate::rich::verify_token(token, &key, client_id)?;
    }
    let json = keys.decrypt(&rich.content)?;
    serde_json::from_slice(&json).map_err(|_| "unexpected message shape".into())
}

/// Microsoft identity platform signing keys, cached; they rotate daily, so an unknown key ID
/// triggers a refresh (at most once a minute).
fn signing_key(client: &Client, kid: &str) -> Result<rsa::RsaPublicKey, Failure> {
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
    type Cache = Option<(Instant, HashMap<String, rsa::RsaPublicKey>)>;
    static CACHE: Mutex<Cache> = Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .map_err(|_| Failure::new("Could not read signing keys."))?;
    if let Some((fetched, keys)) = &*cache {
        if let Some(key) = keys.get(kid) {
            return Ok(key.clone());
        }
        if fetched.elapsed() < Duration::from_secs(60) {
            return Err(Failure::new("Unknown signing key."));
        }
    }
    let jwks: Jwks = decode(
        client
            .get("https://login.microsoftonline.com/common/discovery/keys")
            .send()
            .map_err(http_failure)?,
    )?;
    let keys: HashMap<_, _> = jwks
        .keys
        .into_iter()
        .filter_map(|jwk| Some((jwk.kid, crate::rich::public_key(&jwk.n?, &jwk.e?)?)))
        .collect();
    let key = keys.get(kid).cloned();
    *cache = Some((Instant::now(), keys));
    key.ok_or_else(|| Failure::new("Unknown signing key."))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Subscription {
    id: String,
    resource: String,
    notification_url: String,
    expiration_date_time: String,
}
struct Lease {
    id: String,
    renew_at: Instant,
}
struct Watch {
    registration: Option<Registration>,
    resources: Vec<String>,
    known: HashSet<String>,
    leases: HashMap<String, Lease>,
    cleanup: VecDeque<String>,
    discover: bool,
    retry_at: Instant,
    failed_until: HashMap<String, Instant>,
    attempted: Option<String>,
    /// The user-level `getAllMessages` resource; while it works, per-chat resources are skipped.
    all_messages: String,
    /// Ask Graph to include encrypted messages; cleared for the session if Graph rejects it.
    rich: bool,
}
impl Default for Watch {
    fn default() -> Self {
        Self {
            registration: None,
            resources: Vec::new(),
            known: HashSet::new(),
            leases: HashMap::new(),
            cleanup: VecDeque::new(),
            discover: false,
            retry_at: Instant::now(),
            failed_until: HashMap::new(),
            attempted: None,
            all_messages: String::new(),
            rich: true,
        }
    }
}
impl Watch {
    /// Per-chat subscriptions are needed only when the user-level one was rejected.
    fn wanted(&self, resource: &str) -> bool {
        !resource.starts_with("chats/") || self.failed_until.contains_key(&self.all_messages)
    }
    /// (active, wanted) subscription counts for the status line.
    pub fn coverage(&self) -> (usize, usize) {
        let wanted = self.resources.iter().filter(|r| self.wanted(r));
        let total = wanted.clone().count();
        (
            wanted.filter(|r| self.leases.contains_key(*r)).count(),
            total,
        )
    }
    fn add_resource(&mut self, resource: String) {
        if self.known.insert(resource.clone()) {
            self.resources.push(resource);
        }
    }
    fn configure(&mut self, registration: Option<Registration>) {
        self.registration = registration;
        self.leases.clear();
        self.failed_until.clear();
        self.attempted = None;
        self.cleanup.clear();
        self.discover = self.registration.is_some();
        self.retry_at = Instant::now();
        self.rich = true;
    }
    fn tick(&mut self, client: &Client, session: &mut Session) -> Result<bool, Failure> {
        self.attempted = None;
        let Some(registration) = &self.registration else {
            return Ok(false);
        };
        session.refresh(client)?;
        if self.discover {
            let page: Page<Subscription> = session.get(client, graph_url(&["subscriptions"])?)?;
            let origin = Url::parse(&registration.notification_url)
                .map_err(|_| Failure::new("Invalid relay callback."))?;
            for subscription in page.value {
                let resource = subscription.resource.trim_start_matches('/');
                // This app's delegated listing is scoped to the signed-in user. Replace only this relay's callbacks.
                if Url::parse(&subscription.notification_url).is_ok_and(|url| {
                    url.origin() == origin.origin() && url.path().starts_with("/graph/")
                }) && (resource.starts_with("chats/")
                    || resource.starts_with("chats('")
                    || resource == format!("users/{}/chats", session.user_id)
                    || resource == self.all_messages)
                {
                    self.cleanup.push_back(subscription.id);
                }
            }
            self.discover = false;
            return Ok(false);
        }
        let Some(resource) = self
            .resources
            .iter()
            .find(|resource| {
                self.wanted(resource)
                    && self
                        .failed_until
                        .get(*resource)
                        .is_none_or(|until| *until <= Instant::now())
                    && self
                        .leases
                        .get(*resource)
                        .is_none_or(|lease| lease.renew_at <= Instant::now())
            })
            .cloned()
        else {
            // Old subscriptions are removed only when nothing needs creating, so a backlog of
            // stale ones never delays live updates.
            if let Some(id) = self.cleanup.front().cloned() {
                let response = client
                    .delete(graph_url(&["subscriptions", &id])?)
                    .bearer_auth(session.access_token(client)?)
                    .send()
                    .map_err(http_failure)?;
                if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
                    self.cleanup.pop_front();
                } else {
                    decode::<serde_json::Value>(response)?;
                }
            }
            return Ok(false);
        };
        self.attempted = Some(resource.clone());
        let expiration = (time::OffsetDateTime::now_utc() + time::Duration::minutes(50))
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| Failure::new("Could not calculate subscription expiry."))?;
        let messages = resource.ends_with("/messages") || resource.ends_with("/getAllMessages");
        let keys = (self.rich && messages && !self.leases.contains_key(&resource))
            .then(crate::rich::keys)
            .flatten();
        let response = if let Some(lease) = self.leases.get(&resource) {
            client
                .patch(graph_url(&["subscriptions", &lease.id])?)
                .bearer_auth(session.access_token(client)?)
                .json(&serde_json::json!({"expirationDateTime":expiration}))
                .send()
                .map_err(http_failure)?
        } else {
            let mut body = serde_json::json!({
                "changeType":if messages{"created,updated,deleted"}else{"created,updated"},
                "notificationUrl":registration.notification_url,"lifecycleNotificationUrl":registration.notification_url,
                "resource":resource,"includeResourceData":keys.is_some(),"expirationDateTime":expiration,"clientState":registration.client_state,
                "latestSupportedTlsVersion":"v1_2"
            });
            if let Some(keys) = keys {
                body["encryptionCertificate"] = keys.certificate.clone().into();
                body["encryptionCertificateId"] = keys.id.clone().into();
            }
            client
                .post(graph_url(&["subscriptions"])?)
                .bearer_auth(session.access_token(client)?)
                .json(&body)
                .send()
                .map_err(http_failure)?
        };
        if keys.is_some()
            && response.status().is_client_error()
            && response.status() != StatusCode::TOO_MANY_REQUESTS
        {
            // Fall back to ID-only notifications rather than losing live updates.
            self.rich = false;
            if tracing() {
                eprintln!(
                    "TeamsFast watch: rich notifications rejected (HTTP {}), using plain ones",
                    response.status().as_u16()
                );
            }
            return Ok(false);
        }
        if keys.is_some() && tracing() && response.status().is_success() {
            eprintln!("TeamsFast watch: rich notifications enabled");
        }
        if response.status() == StatusCode::NOT_FOUND && self.leases.remove(&resource).is_some() {
            return Ok(false);
        }
        let subscription: Subscription = decode(response)?;
        let seconds = crate::model::parse_time(&subscription.expiration_date_time)
            .map(|date| {
                (date - time::OffsetDateTime::now_utc())
                    .whole_seconds()
                    .saturating_sub(10 * 60)
                    .max(30) as u64
            })
            .unwrap_or(30 * 60);
        self.failed_until.remove(&resource);
        self.attempted = None;
        self.leases.insert(
            resource,
            Lease {
                id: subscription.id,
                renew_at: Instant::now() + Duration::from_secs(seconds),
            },
        );
        Ok(true)
    }
}

fn sign_in(
    client: &Client,
    config: AccountConfig,
    epoch: u64,
    current: &AtomicU64,
    emit: &impl Fn(Event),
) -> Result<Session, Failure> {
    validate_registration(&config.client_id, &config.tenant)?;
    let response = client
        .post(oauth_url(&config.tenant, "devicecode")?)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("scope", config.scopes()),
        ])
        .send()
        .map_err(http_failure)?;
    let device: DeviceCode = decode(response)?;
    let url = checked_sign_in_url(&device.verification_uri)?;
    emit(Event::DeviceCode {
        user_code: device.user_code,
        url: url.to_string(),
    });
    let deadline = Instant::now() + Duration::from_secs(device.expires_in.min(1800));
    let mut interval = Duration::from_secs(device.interval.max(5));
    loop {
        let next = (Instant::now() + interval).min(deadline);
        while Instant::now() < next {
            if current.load(Ordering::Relaxed) != epoch {
                return Err(Failure::new("Sign-in canceled."));
            }
            thread::sleep(
                Duration::from_millis(100).min(next.saturating_duration_since(Instant::now())),
            );
        }
        if current.load(Ordering::Relaxed) != epoch {
            return Err(Failure::new("Sign-in canceled."));
        }
        if Instant::now() >= deadline {
            return Err(Failure::new(
                "The sign-in code expired. Start sign-in again.",
            ));
        }
        let response = client
            .post(oauth_url(&config.tenant, "token")?)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", config.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
            ])
            .send()
            .map_err(http_failure)?;
        match decode::<Token>(response) {
            Ok(token) => {
                return Ok(Session::from_token(config, token));
            }
            Err(error) if error.code == "authorization_pending" => {}
            Err(error) if error.code == "slow_down" => {
                interval += Duration::from_secs(5);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Authorization code + PKCE through the system browser, redirected to a one-shot loopback
/// listener (`http://localhost:<port>`, registered in Entra as a "Mobile and desktop" redirect).
fn sign_in_browser(
    client: &Client,
    config: AccountConfig,
    epoch: u64,
    current: &AtomicU64,
    emit: &impl Fn(Event),
) -> Result<Session, Failure> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };
    validate_registration(&config.client_id, &config.tenant)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| Failure::new("Could not start the sign-in listener."))?;
    let port = listener
        .local_addr()
        .map_err(|_| Failure::new("Could not start the sign-in listener."))?
        .port();
    // Browsers may resolve localhost to IPv6 first; listen there too when available.
    let listener6 = TcpListener::bind(("::1", port)).ok();
    for listener in std::iter::once(&listener).chain(listener6.as_ref()) {
        listener
            .set_nonblocking(true)
            .map_err(|_| Failure::new("Could not start the sign-in listener."))?;
    }
    let random = |len: usize| -> Result<String, Failure> {
        let mut bytes = vec![0_u8; len];
        getrandom::fill(&mut bytes).map_err(|_| Failure::new("Could not start sign-in."))?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    };
    let verifier = random(32)?;
    let state = random(16)?;
    let redirect = format!("http://localhost:{port}");
    let mut url = oauth_url(&config.tenant, "authorize")?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_mode", "query")
        .append_pair("scope", config.scopes())
        .append_pair("state", &state)
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        )
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(url.as_str())
        .spawn();
    emit(Event::SignInPage(url.to_string()));
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let code = loop {
        if current.load(Ordering::Relaxed) != epoch {
            return Err(Failure::new("Sign-in canceled."));
        }
        if Instant::now() >= deadline {
            return Err(Failure::new("Sign-in timed out. Start sign-in again."));
        }
        let accepted = std::iter::once(&listener)
            .chain(listener6.as_ref())
            .find_map(|listener| listener.accept().ok());
        let Some((mut stream, _)) = accepted else {
            thread::sleep(Duration::from_millis(100));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let query: HashMap<String, String> = Url::parse(&format!("http://localhost{target}"))
            .map(|url| url.query_pairs().into_owned().collect())
            .unwrap_or_default();
        // Favicon and other stray requests: ignore and keep waiting.
        if !query.contains_key("code") && !query.contains_key("error") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let ok = query.get("state") == Some(&state) && query.contains_key("code");
        let page = if ok {
            "Signed in. You can close this tab and return to TeamsFast."
        } else {
            "Sign-in did not complete. Return to TeamsFast and try again."
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>TeamsFast</title><body style=\"font:16px -apple-system,sans-serif;margin:3em\">{page}</body>"
        );
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        if query.get("state") != Some(&state) {
            return Err(Failure::new("Sign-in response did not match. Try again."));
        }
        if let Some(code) = query.get("code") {
            break code.clone();
        }
        // Microsoft's own message (e.g. consent or redirect problems); no secrets in it.
        return Err(Failure::new(&format!(
            "Microsoft sign-in failed: {}",
            query
                .get("error_description")
                .or(query.get("error"))
                .map(String::as_str)
                .unwrap_or("unknown error")
        )));
    };
    let token: Token = decode(
        client
            .post(oauth_url(&config.tenant, "token")?)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", config.client_id.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("code_verifier", verifier.as_str()),
                ("scope", config.scopes()),
            ])
            .send()
            .map_err(http_failure)?,
    )?;
    Ok(Session::from_token(config, token))
}

fn checked_sign_in_url(link: &str) -> Result<Url, Failure> {
    let url = Url::parse(link)
        .map_err(|_| Failure::new("Microsoft returned an invalid sign-in link."))?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "microsoft.com"
                    | "www.microsoft.com"
                    | "login.microsoftonline.com"
                    | "login.microsoft.com"
            )
        )
    {
        return Err(Failure::new(
            "Microsoft returned an unexpected sign-in host.",
        ));
    }
    Ok(url)
}

fn expiry(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds.saturating_sub(60).min(86_400))
}

pub(crate) fn validate_registration(client_id: &str, tenant: &str) -> Result<(), Failure> {
    let uuid = valid_uuid(client_id);
    if !uuid {
        return Err(Failure::new(
            "Enter the Application (client) ID from your Entra app registration.",
        ));
    }
    if tenant.is_empty()
        || tenant.len() > 253
        || !tenant
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
        || tenant.eq_ignore_ascii_case("common")
        || tenant.eq_ignore_ascii_case("consumers")
    {
        return Err(Failure::new(
            "Use a work/school tenant ID, tenant domain, or organizations.",
        ));
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn oauth_url(tenant: &str, endpoint: &str) -> Result<Url, Failure> {
    Url::parse(&format!(
        "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/{endpoint}"
    ))
    .map_err(|_| Failure::new("Invalid Microsoft sign-in address."))
}

fn graph_url(segments: &[&str]) -> Result<Url, Failure> {
    let mut url = Url::parse("https://graph.microsoft.com/v1.0")
        .map_err(|_| Failure::new("Invalid Graph address."))?;
    url.path_segments_mut()
        .map_err(|_| Failure::new("Invalid Graph path."))?
        .extend(segments);
    Ok(url)
}

fn checked_graph_url(link: &str) -> Result<Url, Failure> {
    let url = Url::parse(link).map_err(|_| Failure::new("Invalid pagination link."))?;
    if url.scheme() != "https"
        || url.host_str() != Some("graph.microsoft.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with("/v1.0/")
    {
        return Err(Failure::new(
            "Refusing an unexpected Graph pagination address.",
        ));
    }
    Ok(url)
}

#[derive(Debug)]
pub(crate) struct Failure {
    pub message: String,
    code: String,
    pub retry_after: Option<Duration>,
    pub uncertain: bool,
}

impl Failure {
    fn new(message: &str) -> Self {
        Self {
            message: message.into(),
            code: String::new(),
            retry_after: None,
            uncertain: false,
        }
    }
    fn throttled(delay: Duration) -> Self {
        Self {
            message: format!(
                "Microsoft asked us to wait {} seconds before another request.",
                delay.as_secs().max(1)
            ),
            code: "throttled".into(),
            retry_after: Some(delay),
            uncertain: false,
        }
    }
}

fn http_failure(error: reqwest::Error) -> Failure {
    let mut failure = if error.is_timeout() {
        Failure::new("Microsoft did not respond in time. Check your connection and try again.")
    } else {
        Failure::new(&format!("Network request failed: {}", error.without_url()))
    };
    failure.uncertain = true;
    failure
}

fn decode<T: DeserializeOwned>(response: Response) -> Result<T, Failure> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|header| header.to_str().ok())
        .and_then(|seconds| seconds.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.clamp(1, 3600)));
    let mut bytes = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("Could not read Microsoft's response."))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(Failure::new(
            "Microsoft's response exceeded the size limit.",
        ));
    }
    decode_bytes(status, retry_after, &bytes)
}

fn decode_bytes<T: DeserializeOwned>(
    status: StatusCode,
    retry_after: Option<Duration>,
    bytes: &[u8],
) -> Result<T, Failure> {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Failure::throttled(
            retry_after.unwrap_or(Duration::from_secs(30)),
        ));
    }
    if !status.is_success() {
        let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
        let code = value
            .get("error")
            .and_then(|error| {
                error
                    .as_str()
                    .or_else(|| error.get("code").and_then(|code| code.as_str()))
            })
            .unwrap_or("unknown_error");
        let detail: String = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|message| message.as_str())
            .unwrap_or("Check consent and account access; sign in again if the session expired.")
            .chars()
            .take(500)
            .collect();
        return Err(Failure {
            message: format!("Microsoft returned {} ({code}): {detail}", status.as_u16()),
            code: code.into(),
            retry_after,
            uncertain: status.is_server_error(),
        });
    }
    serde_json::from_slice(bytes)
        .map_err(|_| Failure::new("Microsoft returned an unreadable response."))
}

#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}
fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    id: String,
    display_name: String,
}

#[derive(Deserialize)]
struct Page<T> {
    value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphChat {
    id: String,
    chat_type: Option<String>,
    created_date_time: Option<String>,
    last_updated_date_time: Option<String>,
    topic: Option<String>,
    web_url: Option<String>,
    last_message_preview: Option<GraphPreview>,
    viewpoint: Option<GraphViewpoint>,
    #[serde(default)]
    members: Vec<Member>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphViewpoint {
    #[serde(default)]
    is_hidden: bool,
    last_message_read_date_time: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphPreview {
    id: Option<String>,
    message_type: Option<String>,
    event_detail: Option<EventDetail>,
    #[serde(default)]
    is_deleted: bool,
    body: Option<Body>,
    created_date_time: Option<String>,
    from: Option<Author>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    user_id: Option<String>,
    display_name: Option<String>,
}

impl GraphChat {
    fn summary(self, me: &str) -> ChatSummary {
        let members = self.members.len();
        // 1:1 chats show the other person's photo; groups keep initials.
        let avatar_user = (members == 2)
            .then(|| {
                self.members
                    .iter()
                    .find_map(|member| member.user_id.clone().filter(|id| id != me))
            })
            .flatten();
        // Exact IDs only: member lists are not reliable enough to infer a chat with yourself.
        let is_self = self.id == SELF_CHAT || self.id == format!("19:{me}_{me}@unq.gbl.spaces");
        let preview = self.last_message_preview.as_ref();
        let has_message_preview = preview.is_some_and(|p| {
            p.message_type.as_deref() == Some("message") && p.event_detail.is_none()
        });
        let preview_is_message = has_message_preview && preview.is_some_and(|p| !p.is_deleted);
        let preview_text = if preview.is_some_and(|p| p.is_deleted) {
            "Message deleted".into()
        } else if preview_is_message {
            preview
                .and_then(|p| p.body.as_ref())
                .map(body_text)
                .unwrap_or_default()
                .replace('\n', " ")
        } else if preview.is_some() {
            "Conversation activity".into()
        } else {
            String::new()
        };
        let preview_id = preview.and_then(|p| p.id.clone());
        let preview_mine = preview
            .and_then(|p| p.from.as_ref())
            .and_then(|p| p.user.as_ref())
            .and_then(|p| p.id.as_deref())
            == Some(me);
        // Membership/rename dates must not make an old message look newly unread.
        let updated_at = preview
            .and_then(|p| p.created_date_time.as_ref())
            .into_iter()
            .chain(
                self.last_updated_date_time
                    .as_ref()
                    .filter(|_| !preview_is_message),
            )
            .chain(
                self.created_date_time
                    .as_ref()
                    .filter(|_| !preview_is_message),
            )
            .max_by_key(|date| crate::model::timestamp(date))
            .cloned()
            .unwrap_or_default();
        let preview_sender = preview
            .and_then(|p| p.from.as_ref())
            .and_then(|from| from.user.as_ref())
            .filter(|user| user.id.as_deref() != Some(me))
            .and_then(|user| user.display_name.clone());
        let my_name = self
            .members
            .iter()
            .find(|member| member.user_id.as_deref() == Some(me))
            .and_then(|member| member.display_name.clone());
        let title = self
            .topic
            .filter(|topic| !topic.trim().is_empty() && !is_self)
            .unwrap_or_else(|| {
                if is_self {
                    return format!("{} (You)", my_name.as_deref().unwrap_or("Notes"));
                }
                let names: Vec<_> = self
                    .members
                    .into_iter()
                    .filter(|member| member.user_id.as_deref() != Some(me))
                    .filter_map(|member| member.display_name)
                    .filter(|name| !name.trim().is_empty())
                    .collect();
                if names.is_empty() {
                    // Graph omits some members (e.g. people from other organizations); the
                    // latest sender still names a 1:1 chat when it was not me.
                    preview_sender.unwrap_or_else(|| "Untitled chat".into())
                } else {
                    names.join(", ")
                }
            });
        ChatSummary {
            id: self.id,
            title,
            preview: preview_text,
            updated_at,
            preview_id,
            preview_mine: preview_is_message && preview_mine,
            preview_is_message,
            hidden: self.viewpoint.as_ref().is_some_and(|v| v.is_hidden),
            unavailable: false,
            is_meeting: self.chat_type.as_deref() == Some("meeting"),
            has_messages: has_message_preview.then_some(true),
            web_url: self.web_url,
            read_at: self.viewpoint.and_then(|v| v.last_message_read_date_time),
            members,
            avatar_user,
            is_self,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMessage {
    id: String,
    message_type: Option<String>,
    event_detail: Option<EventDetail>,
    created_date_time: String,
    last_modified_date_time: Option<String>,
    body: Body,
    from: Option<Author>,
    deleted_date_time: Option<String>,
    #[serde(default)]
    attachments: Vec<Attachment>,
    #[serde(default)]
    reactions: Vec<GraphReaction>,
}

#[derive(Deserialize)]
struct EventDetail {
    #[serde(rename = "@odata.type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    content_type: String,
    content: String,
}
#[derive(Deserialize)]
struct Author {
    user: Option<Identity>,
    application: Option<Identity>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    id: Option<String>,
    display_name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attachment {
    content_type: Option<String>,
    content_url: Option<String>,
    content: Option<String>,
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphReaction {
    reaction_type: String,
    display_name: Option<String>,
    user: Option<ReactionUser>,
}
#[derive(Deserialize)]
struct ReactionUser {
    user: Option<Identity>,
}
/// `content` of a `messageReference` attachment (the message a reply quotes).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageReference {
    message_preview: Option<String>,
    message_sender: Option<Author>,
}

fn body_text(body: &Body) -> String {
    body_views(body).0
}

fn body_views(body: &Body) -> (String, String) {
    use html2text::render::{RichAnnotation, TaggedLineElement};
    if !body.content_type.eq_ignore_ascii_case("html") {
        return (
            body.content.clone(),
            crate::model::message_markdown(&body.content),
        );
    }
    let Ok(lines) = html2text::from_read_rich(body.content.as_bytes(), 10_000) else {
        return (
            "This formatted message could not be displayed.".into(),
            String::new(),
        );
    };
    let mut plain = String::new();
    let mut markdown = String::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            plain.push('\n');
            markdown.push_str("  \n");
        }
        for segment in line {
            let TaggedLineElement::Str(segment) = segment else {
                continue;
            };
            plain.push_str(&segment.s);
            let text = crate::model::escape_markdown(&segment.s);
            let link = segment.tag.iter().find_map(|tag| match tag {
                RichAnnotation::Link(url) => reqwest::Url::parse(url)
                    .ok()
                    .filter(|url| matches!(url.scheme(), "https" | "http")),
                _ => None,
            });
            if let Some(link) = link {
                let text = crate::model::link_label(&segment.s);
                let target = link
                    .as_str()
                    .replace('(', "%28")
                    .replace(')', "%29")
                    .replace(' ', "%20");
                markdown.push_str(&format!("[{text}]({target})"));
            } else if segment.tag.contains(&RichAnnotation::Strong) {
                markdown.push_str(&format!("**{text}**"));
            } else {
                markdown.push_str(&text);
            }
        }
    }
    (plain.trim().to_owned(), markdown.trim().to_owned())
}

impl GraphMessage {
    fn is_system_event(&self) -> bool {
        self.event_detail.is_some()
            || matches!(
                self.message_type.as_deref(),
                Some("systemEventMessage" | "chatEvent")
            )
    }

    fn display(self, me: &str) -> Message {
        use crate::model::{File, Quote, Reaction, reaction_emoji};
        let user = self.from.as_ref().and_then(|author| author.user.as_ref());
        let author = user
            .or_else(|| {
                self.from
                    .as_ref()
                    .and_then(|author| author.application.as_ref())
            })
            .and_then(|author| author.display_name.clone())
            .unwrap_or_else(|| "Teams".into());
        let deleted = self.deleted_date_time.is_some();
        let system_event = self.is_system_event();
        let mut message = Message {
            id: self.id,
            author,
            author_id: user.and_then(|user| user.id.clone()).unwrap_or_default(),
            modified_at: self
                .last_modified_date_time
                .unwrap_or_else(|| self.created_date_time.clone()),
            created_at: self.created_date_time,
            mine: user.and_then(|user| user.id.as_deref()) == Some(me),
            deleted,
            system: self
                .message_type
                .as_deref()
                .is_some_and(|kind| kind != "message")
                || self.event_detail.is_some(),
            ..Default::default()
        };
        if message.system {
            // Event XML is not message HTML. Keep a quiet activity row, never a user bubble.
            if system_event {
                message.text = match self.event_detail.and_then(|detail| detail.kind).as_deref() {
                    Some("#microsoft.graph.membersAddedEventMessageDetail") => "Participants added",
                    Some("#microsoft.graph.membersDeletedEventMessageDetail") => {
                        "Participants removed"
                    }
                    Some("#microsoft.graph.chatRenamedEventMessageDetail") => "Chat renamed",
                    Some("#microsoft.graph.callStartedEventMessageDetail") => "Call started",
                    Some("#microsoft.graph.callEndedEventMessageDetail") => "Call ended",
                    _ => "Conversation updated",
                }
                .into();
            }
            return message;
        }
        if deleted {
            message.text = "Message deleted".into();
            message.markdown = "Message deleted".into();
            return message;
        }
        (message.text, message.markdown) = body_views(&self.body);
        if self.body.content_type.eq_ignore_ascii_case("html") {
            message.html = crate::html::teams_html(&self.body.content);
            message.markdown.clear();
            (message.images, message.link) = crate::html::media(&message.html);
        } else {
            message.link = linkify::LinkFinder::new()
                .links(&message.text)
                .map(|link| link.as_str().to_owned())
                .find(|link| link.starts_with("https://"));
        }
        for attachment in self.attachments {
            match attachment.content_type.as_deref() {
                Some("messageReference") => {
                    let reference: Option<MessageReference> = attachment
                        .content
                        .as_deref()
                        .and_then(|content| serde_json::from_str(content).ok());
                    message.quote = reference.map(|reference| Quote {
                        author: reference
                            .message_sender
                            .as_ref()
                            .and_then(|sender| sender.user.as_ref().or(sender.application.as_ref()))
                            .and_then(|sender| sender.display_name.clone())
                            .unwrap_or_default(),
                        text: reference.message_preview.unwrap_or_default(),
                    });
                }
                kind => {
                    let name = attachment.name.unwrap_or_else(|| {
                        if kind.is_some_and(|kind| kind.contains("card")) {
                            "Card".into()
                        } else {
                            "Attachment".into()
                        }
                    });
                    message.text.push_str(&format!("\n[Attachment: {name}]"));
                    message.files.push(File {
                        name,
                        url: attachment
                            .content_url
                            .filter(|url| kind == Some("reference") && url.starts_with("https://")),
                    });
                }
            }
        }
        for reaction in self.reactions {
            let mine = reaction
                .user
                .and_then(|user| user.user)
                .and_then(|user| user.id)
                .as_deref()
                == Some(me);
            if let Some(existing) = message
                .reactions
                .iter_mut()
                .find(|existing| existing.kind == reaction.reaction_type)
            {
                existing.count += 1;
                existing.mine |= mine;
            } else {
                let emoji = match reaction_emoji(&reaction.reaction_type) {
                    // Custom tenant emoji arrive as "custom" with a display name.
                    "custom" => reaction.display_name.unwrap_or_else(|| "★".into()),
                    emoji => emoji.to_owned(),
                };
                message.reactions.push(Reaction {
                    kind: reaction.reaction_type,
                    emoji,
                    count: 1,
                    mine,
                });
            }
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_and_legacy_microsoft_sign_in_hosts_are_allowed_without_spoofing() {
        for link in [
            "https://login.microsoft.com/device",
            "https://microsoft.com/devicelogin",
            "https://www.microsoft.com/devicelogin",
            "https://login.microsoftonline.com/common/oauth2/deviceauth",
        ] {
            assert!(checked_sign_in_url(link).is_ok(), "{link}");
        }
        for link in [
            "http://login.microsoft.com/device",
            "https://login.microsoft.com.evil.example/device",
            "https://evil.example/device",
            "https://login.microsoft.com:444/device",
            "https://user:password@login.microsoft.com/device",
        ] {
            assert!(checked_sign_in_url(link).is_err(), "{link}");
        }
    }

    #[test]
    fn pagination_and_path_segments_cannot_leak_bearer_tokens() {
        for url in [
            "http://graph.microsoft.com/v1.0/me",
            "https://evil.example/v1.0/me",
            "https://graph.microsoft.com.evil.example/v1.0/me",
            "https://graph.microsoft.com:444/v1.0/me",
            "https://user@graph.microsoft.com/v1.0/me",
            "https://graph.microsoft.com/beta/me",
        ] {
            assert!(checked_graph_url(url).is_err(), "{url}");
        }
        assert!(
            checked_graph_url("https://graph.microsoft.com/v1.0/me/chats?$skiptoken=opaque")
                .is_ok()
        );
        let url = graph_url(&["chats", "19:chat/with?reserved#chars", "messages"]).unwrap();
        assert!(url.path().contains("%2F"));
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }

    #[test]
    fn registration_rejects_personal_accounts_and_path_injection() {
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        assert!(validate_registration(id, "organizations").is_ok());
        for tenant in [
            "",
            "common",
            "consumers",
            "tenant/oauth2",
            "https://evil.example",
        ] {
            assert!(validate_registration(id, tenant).is_err());
        }
        assert!(validate_registration("not-a-client-id", "organizations").is_err());
    }

    #[test]
    fn oauth_pending_and_throttling_remain_distinct() {
        let pending = decode_bytes::<Token>(
            StatusCode::BAD_REQUEST,
            None,
            br#"{"error":"authorization_pending"}"#,
        )
        .err()
        .unwrap();
        assert_eq!(pending.code, "authorization_pending");
        let limited = decode_bytes::<User>(
            StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(42)),
            b"{}",
        )
        .err()
        .unwrap();
        assert_eq!(limited.retry_after, Some(Duration::from_secs(42)));
        assert!(decode_bytes::<User>(StatusCode::OK, None, b"invalid json").is_err());
        let denied = decode_bytes::<User>(
            StatusCode::BAD_REQUEST,
            None,
            br#"{"error":{"code":"ValidationError","message":"Notification endpoint validation failed."}}"#,
        ).err().unwrap();
        assert!(
            denied
                .message
                .contains("Notification endpoint validation failed.")
        );
    }

    #[test]
    fn graph_html_deleted_messages_and_attachments_are_readable() {
        let json = br#"{"id":"m1","createdDateTime":"2026-10-07T08:00:00Z","body":{"contentType":"html","content":"<p>Hello &amp; <b>world</b><br>Next line</p>"},"from":{"user":{"id":"me","displayName":"You"}},"attachments":[{"name":"notes.txt"}]}"#;
        let message: GraphMessage = serde_json::from_slice(json).unwrap();
        let message = message.display("me");
        assert!(message.mine);
        assert!(message.text.contains("Hello &"));
        assert!(message.text.contains("world"));
        assert!(message.text.contains("notes.txt"));
        assert!(!message.text.contains("<p>"));
        let mut json: serde_json::Value = serde_json::from_slice(json).unwrap();
        json["deletedDateTime"] = "2026-10-07T09:00:00Z".into();
        let deleted: GraphMessage = serde_json::from_value(json).unwrap();
        assert_eq!(deleted.display("me").text, "Message deleted");
    }
}
