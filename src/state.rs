use crate::Wake;
use crate::{
    model::{
        self, Chat, ChatSummary, Message, MessageChange, NoticeHistory, PendingChange, Person,
        Preview, demo_chats,
    },
    notifications::{NoticeEvent, Notifications},
    realtime::{PushClient, PushEvent},
    settings::Settings,
    store::{self, Store},
    teams::{AccountConfig, Command, Event, LoadKind, Media, Operation, Worker},
};
use gpui_kit::{Image, ImageFormat, RenderImage};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::UtcOffset;

/// Decoded message images kept in memory (and GPU textures). The open conversation is exempt.
const MEDIA_BUDGET: usize = 48 * 1024 * 1024;

#[derive(PartialEq)]
pub(crate) enum Mode {
    Demo,
    SigningIn,
    Offline,
    Live,
}

#[derive(Default)]
pub(crate) struct NewChat {
    pub query: String,
    pub people: Vec<Person>,
    pub selected: Vec<Person>,
    pub topic: String,
    pub loading: bool,
    pub creating: bool,
    pub search_at: Option<Instant>,
    pub error: Option<String>,
}

pub(crate) struct ChatState {
    pub worker: Option<Worker>,
    store: Option<Store>,
    pub store_ready: bool,
    cache_dirty: HashSet<String>,
    cache_removed: HashMap<String, Vec<String>>,
    pub draft_dirty: HashSet<String>,
    read_active: Option<(String, Arc<AtomicBool>)>,
    hot_chats: VecDeque<String>,
    pub mode: Mode,
    pub chats: Vec<Chat>,
    pub selected: Option<String>,
    pub search: String,
    pub name: String,
    pub user_id: String,
    pub account: String,
    pub prefs: Settings,
    pub next_chats: Option<String>,
    pub chats_loading: bool,
    pub baseline_ready: bool,
    pub quiet_chat_sync: bool,
    pub error: Option<String>,
    pub disconnect_open: bool,
    pub device_code: Option<(String, String)>,
    pub new_chat: Option<NewChat>,
    pub demo_message_id: u64,
    pub timelines: HashMap<String, Timeline>,
    pub offset: UtcOffset,
    pub force_demo: bool,
    pub push: Option<PushClient>,
    /// Browser sign-in page, so it can be reopened while waiting.
    pub sign_in_page: Option<String>,
    pub relay_status: String,
    pub relay_connected: bool,
    pub watch_active: usize,
    pub watch_total: usize,
    pub watch_error: Option<String>,
    pub watch_pending: Option<teamsfast_relay::Registration>,
    pub notices: Option<Notifications>,
    pub notice_history: NoticeHistory,
    /// (notify, deleted, encrypted message from a rich notification)
    pub pending_changes: BTreeMap<(String, String), (bool, bool, Option<crate::teams::Rich>)>,
    pub change_inflight: bool,
    /// Unread count last shown on the app icon.
    pub badge: usize,
    pub retry_at: Instant,
    pub wake: Wake,
    pub focused: bool,
    pub focus_requested: bool,
    pub revision: u64,
    pub can_save_settings: bool,
    /// Profile photos by Graph user ID; `None` while loading or when the person has none.
    pub photos: HashMap<String, Option<Arc<Image>>>,
    /// Message images by URL, decoded at display size; `None` while loading or when unavailable.
    /// Shared with renderers.
    pub media: Arc<HashMap<String, Option<Arc<RenderImage>>>>,
    pub previews: Arc<HashMap<String, Option<Preview>>>,
    /// Loaded images, oldest first, with their decoded size; bounded by `MEDIA_BUDGET`.
    media_order: VecDeque<(String, usize)>,
    media_bytes: usize,
    /// Image URLs of the open conversation, never evicted while it is open.
    media_wanted: HashSet<String>,
    /// Chat and revision `media_wanted` was computed for.
    media_scanned: Option<(String, u64)>,
    /// Evicted images whose GPU textures the window must release.
    pub evicted: Vec<Arc<RenderImage>>,
    /// Set when media arrives so transcripts remeasure rows that display it.
    pub media_changed: bool,
    /// Push arrival per new message ID, for the delivery trace only.
    push_times: HashMap<String, time::OffsetDateTime>,
}

#[derive(Default)]
pub(crate) struct Timeline {
    pub at_bottom: bool,
    pub jump_to_bottom: bool,
    pub preserve_anchor: bool,
}

impl ChatState {
    pub fn new(force_demo: bool, wake: Wake) -> Self {
        let mut app = Self::demo();
        app.force_demo = force_demo;
        app.wake = wake.clone();
        app.offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        if !force_demo {
            match Settings::load() {
                Ok(settings) => app.prefs = settings,
                Err(error) => {
                    app.error = Some(error);
                    app.can_save_settings = false;
                }
            }
        }
        if !force_demo {
            app.chats.clear();
            app.selected = None;
        }
        app.worker = Some(Worker::new(wake.clone()));
        app.notices = Some(Notifications::new(wake));
        if !force_demo && app.prefs.remember && !app.prefs.client_id.is_empty() {
            app.mode = Mode::SigningIn;
            app.chats.clear();
            app.selected = None;
            let prefix = format!("{}/{}/", app.prefs.tenant, app.prefs.client_id);
            if app.prefs.cached_account.starts_with(&prefix) {
                app.account = app.prefs.cached_account.clone();
                app.open_store();
            }
            app.request(Command::Resume(app.account_config()));
        }
        app
    }

    pub fn demo() -> Self {
        let chats = demo_chats();
        let selected = chats.first().map(|chat| chat.summary.id.clone());
        Self {
            worker: None,
            store: None,
            store_ready: false,
            cache_dirty: HashSet::new(),
            cache_removed: HashMap::new(),
            draft_dirty: HashSet::new(),
            read_active: None,
            hot_chats: VecDeque::new(),
            mode: Mode::Demo,
            chats,
            selected,
            search: String::new(),
            name: String::new(),
            user_id: String::new(),
            account: String::new(),
            prefs: Settings {
                tenant: "organizations".into(),
                ..Default::default()
            },
            next_chats: None,
            chats_loading: false,
            baseline_ready: false,
            quiet_chat_sync: false,
            error: None,
            disconnect_open: false,
            device_code: None,
            new_chat: None,
            demo_message_id: 0,
            timelines: HashMap::new(),
            offset: UtcOffset::UTC,
            force_demo: true,
            push: None,
            sign_in_page: None,
            relay_status: "Live updates not configured".into(),
            relay_connected: false,
            watch_active: 0,
            watch_total: 0,
            watch_error: None,
            watch_pending: None,
            notices: None,
            notice_history: NoticeHistory::default(),
            pending_changes: BTreeMap::new(),
            badge: 0,
            change_inflight: false,
            retry_at: Instant::now(),
            wake: async_channel::bounded(1).0,
            focused: false,
            focus_requested: false,
            revision: 0,
            can_save_settings: true,
            photos: HashMap::new(),
            media: Arc::default(),
            previews: Arc::default(),
            media_order: VecDeque::new(),
            media_bytes: 0,
            media_wanted: HashSet::new(),
            media_scanned: None,
            evicted: Vec::new(),
            media_changed: false,
            push_times: HashMap::new(),
        }
    }

    pub fn account_config(&self) -> AccountConfig {
        AccountConfig {
            client_id: self.prefs.client_id.trim().into(),
            tenant: self.prefs.tenant.trim().into(),
            remember: true,
            use_code: false,
        }
    }
    pub fn photo(&self, user_id: Option<&str>) -> Option<Arc<Image>> {
        self.photos.get(user_id?).cloned().flatten()
    }
    fn request_photo(&mut self, user_id: &str) {
        if self.mode == Mode::Live
            && !user_id.is_empty()
            && !self.photos.contains_key(user_id)
            && let Some(worker) = &self.worker
        {
            self.photos.insert(user_id.to_owned(), None);
            worker.media(Media::Photo(user_id.to_owned()));
        }
    }
    pub fn epoch(&self) -> u64 {
        self.worker.as_ref().map_or(0, Worker::epoch)
    }
    pub fn request(&mut self, command: Command) -> bool {
        match self
            .worker
            .as_ref()
            .ok_or_else(|| "The network worker is unavailable.".to_owned())
            .and_then(|worker| worker.send(command))
        {
            Ok(()) => {
                self.revision = self.revision.wrapping_add(1);
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }
    pub fn save_drafts(&mut self) {
        let Some(store) = self.store.as_ref().filter(|_| self.store_ready) else {
            return;
        };
        self.draft_dirty.retain(|id| {
            let Some(chat) = self.chats.iter().find(|c| &c.summary.id == id) else {
                return false;
            };
            // The prepare transaction clears the old draft only after the send text is durable.
            if chat.draft.is_empty()
                && chat
                    .messages
                    .iter()
                    .any(|m| m.delivery == model::Delivery::Sending)
            {
                return true;
            }
            store
                .send(store::Command::Draft {
                    chat_id: id.clone(),
                    text: chat.draft.clone(),
                })
                .is_err()
        });
    }
    pub fn is_muted(&self, id: &str) -> bool {
        self.prefs
            .muted
            .get(&self.account)
            .is_some_and(|set| set.contains(id))
    }
    pub fn toggle_mute(&mut self, id: String) {
        let muted = self.prefs.muted.entry(self.account.clone()).or_default();
        if !muted.remove(&id) {
            muted.insert(id);
        }
    }
    pub fn use_demo(&mut self, forget: bool) {
        self.save_drafts();
        self.cancel_read();
        self.store = None;
        self.store_ready = false;
        self.cache_dirty.clear();
        self.cache_removed.clear();
        self.draft_dirty.clear();
        let config = self.account_config();
        if let Some(worker) = &self.worker {
            worker.reset();
        }
        if forget {
            self.prefs.remember = false;
            self.prefs.cached_account.clear();
            self.request(Command::Forget(config));
        }
        self.push = None;
        self.watch_pending = None;
        self.pending_changes.clear();
        self.change_inflight = false;
        self.mode = Mode::Demo;
        // Signed out shows only the sign-in screen; sample chats are for `--demo`.
        self.chats = if self.force_demo {
            demo_chats()
        } else {
            Vec::new()
        };
        self.selected = self.chats.first().map(|chat| chat.summary.id.clone());
        self.sign_in_page = None;
        self.search.clear();
        self.next_chats = None;
        self.chats_loading = false;
        self.baseline_ready = false;
        self.name.clear();
        self.user_id.clear();
        self.account.clear();
        self.device_code = None;
        self.error = None;
        self.disconnect_open = false;
        self.timelines.clear();
        self.relay_connected = false;
        self.watch_active = 0;
        self.watch_total = 0;
        self.watch_error = None;
        self.notice_history = NoticeHistory::default();
    }

    pub fn start_sign_in(&mut self, use_code: bool) {
        let config = AccountConfig {
            use_code,
            ..self.account_config()
        };
        self.prefs.remember = true;
        if let Err(error) = crate::teams::validate_registration(&config.client_id, &config.tenant) {
            self.error = Some(error.message);
            return;
        }
        self.save_drafts();
        self.cancel_read();
        self.store = None;
        self.store_ready = false;
        self.cache_dirty.clear();
        self.cache_removed.clear();
        self.draft_dirty.clear();
        self.push = None;
        if let Some(worker) = &self.worker {
            worker.reset();
        }
        if self.request(Command::SignIn(config)) {
            self.mode = Mode::SigningIn;
            self.chats.clear();
            self.selected = None;
            self.search.clear();
            self.account.clear();
            self.device_code = None;
            self.sign_in_page = None;
            self.error = None;
            self.baseline_ready = false;
            self.watch_pending = None;
            self.pending_changes.clear();
            self.change_inflight = false;
            self.timelines.clear();
        }
    }

    pub fn start_push(&mut self) {
        self.push = None;
        self.watch_pending = None;
        self.relay_connected = false;
        self.watch_active = 0;
        self.watch_total = 0;
        self.watch_error = None;
        self.request(Command::Watch(None));
        if self.prefs.relay_url.trim().is_empty() {
            self.relay_status = "Automatic refresh".into();
            return;
        }
        if let Err(error) = crate::realtime::base_url(self.prefs.relay_url.trim()) {
            self.relay_status = error;
            return;
        }
        let Some(worker) = &self.worker else {
            return;
        };
        self.relay_status = "Connecting live updates…".into();
        self.push = Some(PushClient::start(
            self.prefs.relay_url.trim().into(),
            worker.relay_credential(),
            self.wake.clone(),
        ));
    }

    pub fn receive(&mut self) {
        loop {
            let event = self
                .worker
                .as_ref()
                .and_then(|worker| worker.events.try_recv().ok());
            let Some((epoch, event)) = event else {
                break;
            };
            if epoch != self.epoch() {
                continue;
            }
            self.revision = self.revision.wrapping_add(1);
            self.apply_event(event);
        }
        loop {
            let event = self
                .push
                .as_ref()
                .and_then(|push| push.events.try_recv().ok());
            let Some(event) = event else {
                break;
            };
            self.revision = self.revision.wrapping_add(1);
            match event {
                PushEvent::Ready(registration) => {
                    self.watch_pending = Some(registration);
                    self.watch_active = 0;
                    self.watch_total = 0;
                    self.watch_error = None;
                }
                PushEvent::Status(text, connected) if crate::teams::tracing() => {
                    eprintln!("TeamsFast relay: connected={connected} ({text})");
                    self.relay_status = text;
                    self.relay_connected = connected;
                }
                PushEvent::Status(text, connected) => {
                    self.relay_status = text;
                    self.relay_connected = connected;
                }
                PushEvent::Changes(batch, recovered) => {
                    // Counts and kinds only: no chat/message IDs or content.
                    if crate::teams::tracing() {
                        let kinds: Vec<_> = batch
                            .changes
                            .iter()
                            .map(|change| match (&change.chat_id, &change.message_id) {
                                (Some(_), Some(_)) => change.kind.as_str(),
                                _ => "chat-list",
                            })
                            .collect();
                        eprintln!(
                            "TeamsFast push: {} change(s) {kinds:?} reset={} recovered={recovered}",
                            kinds.len(),
                            batch.reset
                        );
                    }
                    if batch.reset || recovered {
                        self.quiet_chat_sync = true;
                        if !self.chats_loading {
                            self.refresh_chats(None, true);
                        }
                        if let Some(id) = self.selected.clone() {
                            self.refresh_messages(id, None, LoadKind::Catchup);
                        }
                    }
                    for change in batch.changes {
                        if change.kind == "resync" {
                            self.quiet_chat_sync = true;
                            self.request(Command::Renew);
                            if !self.chats_loading {
                                self.refresh_chats(None, true);
                            }
                            continue;
                        }
                        match (change.chat_id, change.message_id) {
                            (Some(chat), Some(message)) => {
                                if crate::teams::tracing() && change.kind == "created" {
                                    if self.push_times.len() > 512 {
                                        self.push_times.clear();
                                    }
                                    self.push_times
                                        .insert(message.clone(), time::OffsetDateTime::now_utc());
                                }
                                if self.pending_changes.len() < 512 {
                                    // The newest content wins; a change without it forces a fetch.
                                    let rich = change.content.map(|content| crate::teams::Rich {
                                        content,
                                        tokens: change.tokens,
                                    });
                                    let created = change.kind == "created" && !recovered;
                                    let deleted = change.kind == "deleted";
                                    self.pending_changes
                                        .entry((chat, message))
                                        .and_modify(|value| {
                                            value.0 |= created;
                                            value.1 = deleted;
                                            value.2 = rich.clone();
                                        })
                                        .or_insert((created, deleted, rich));
                                } else if !self.chats_loading {
                                    self.refresh_chats(None, true);
                                }
                            }
                            _ => {
                                if !self.chats_loading {
                                    self.refresh_chats(None, false);
                                }
                            }
                        }
                    }
                }
            }
        }
        loop {
            let event = self
                .notices
                .as_ref()
                .and_then(|notices| notices.events.try_recv().ok());
            let Some(event) = event else {
                break;
            };
            self.revision = self.revision.wrapping_add(1);
            match event {
                NoticeEvent::Open { epoch, chat_id } if epoch == self.epoch() => {
                    self.focus_requested = true;
                    if self.chats.iter().any(|chat| chat.summary.id == chat_id) {
                        self.select_chat(chat_id.clone());
                        self.timelines.entry(chat_id).or_default().jump_to_bottom = true;
                    }
                }
                NoticeEvent::Error(error) => self.error = Some(error),
                _ => {}
            }
        }
    }

    pub fn apply_event(&mut self, event: Event) {
        match event {
            Event::DeviceCode { user_code, url } => self.device_code = Some((user_code, url)),
            Event::SignInPage(url) => self.sign_in_page = Some(url),
            Event::Connected { user_id, name } => {
                let account = format!("{}/{}/{}", self.prefs.tenant, self.prefs.client_id, user_id);
                if self.account != account {
                    self.chats.clear();
                    self.selected = None;
                    self.store = None;
                    self.store_ready = false;
                    self.cache_dirty.clear();
                    self.cache_removed.clear();
                    self.draft_dirty.clear();
                }
                self.user_id = user_id;
                self.name = name;
                self.account = account;
                self.prefs.cached_account = self.account.clone();
                if self.store.is_none() {
                    self.open_store();
                } else if let Some(store) = &self.store {
                    let _ = store.send(store::Command::Name(self.name.clone()));
                }
                self.mode = Mode::Live;
                self.device_code = None;
                self.sign_in_page = None;
                self.baseline_ready = false;
                self.notice_history = NoticeHistory::default();
                self.photos.clear();
                self.refresh_chats(None, false);
                self.start_push();
                if let Some(id) = self.selected.clone() {
                    self.refresh_messages(id, None, LoadKind::Initial);
                }
            }
            Event::Chats {
                chats,
                next,
                background,
                started_revision,
            } => {
                self.chats_loading = false;
                self.next_chats = next.clone();
                for user in chats.iter().filter_map(|chat| chat.avatar_user.clone()) {
                    self.request_photo(&user);
                }
                for mut summary in chats {
                    if summary.is_self && !self.name.is_empty() {
                        summary.title = format!("{} (You)", self.name);
                    }
                    if let Some(chat) = self
                        .chats
                        .iter_mut()
                        .find(|chat| chat.summary.id == summary.id)
                    {
                        if self.baseline_ready
                            && summary.preview_id.is_some()
                            && summary.preview_id != chat.summary.preview_id
                            && let Some(id) = summary.preview_id.clone()
                        {
                            self.pending_changes
                                .entry((summary.id.clone(), id))
                                .or_insert((
                                    !summary.preview_mine && !self.quiet_chat_sync,
                                    false,
                                    None,
                                ));
                        }
                        if chat.read_revision > started_revision {
                            summary.read_at.clone_from(&chat.summary.read_at);
                        }
                        if model::timestamp(&summary.updated_at)
                            < model::timestamp(&chat.summary.updated_at)
                        {
                            summary.updated_at.clone_from(&chat.summary.updated_at);
                            summary.preview.clone_from(&chat.summary.preview);
                            summary.preview_id.clone_from(&chat.summary.preview_id);
                            summary.preview_mine = chat.summary.preview_mine;
                        }
                        chat.summary = summary;
                        chat.recount_unread();
                    } else {
                        let mut chat = Chat::new(summary);
                        chat.draft = self
                            .prefs
                            .drafts
                            .get(&self.account)
                            .and_then(|drafts| drafts.get(&chat.summary.id))
                            .cloned()
                            .unwrap_or_default();
                        chat.recount_unread();
                        self.chats.push(chat);
                    }
                }
                self.cache_dirty
                    .extend(self.chats.iter().map(|c| c.summary.id.clone()));
                self.sort_chats();
                // Open the latest conversation, not the pinned (often empty) self chat.
                if self.selected.is_none()
                    && let Some(id) = self
                        .chats
                        .iter()
                        .find(|chat| !chat.summary.is_self)
                        .or(self.chats.first())
                        .map(|chat| chat.summary.id.clone())
                {
                    self.select_chat(id);
                }
                if !background && let Some(next) = next {
                    self.refresh_chats(Some(next), false);
                } else {
                    self.baseline_ready = true;
                    self.quiet_chat_sync = false;
                }
            }
            Event::Messages {
                chat_id,
                messages,
                next,
                kind,
                started_revision,
            } => {
                for message in messages.iter().filter(|message| !message.mine) {
                    self.request_photo(&message.author_id);
                }
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.summary.id == chat_id)
                {
                    if kind == LoadKind::Older {
                        self.timelines
                            .entry(chat_id.clone())
                            .or_default()
                            .preserve_anchor = true;
                    }
                    if kind == LoadKind::Initial || kind == LoadKind::Older || !chat.loaded {
                        chat.next_messages = next;
                    }
                    if kind == LoadKind::Initial {
                        for message in &messages {
                            self.notice_history.first(&chat_id, &message.id);
                        }
                    }
                    for message in &messages {
                        if chat
                            .pending
                            .get(&message.id)
                            .is_some_and(|p| p.uncertain && p.revision <= started_revision)
                        {
                            chat.pending.remove(&message.id);
                            chat.revision = chat.revision.wrapping_add(1);
                        }
                    }
                    chat.merge_messages_after(messages, Some(started_revision));
                    chat.recount_unread();
                    self.cache_dirty.insert(chat_id.clone());
                    chat.loading = false;
                    chat.loaded = true;
                }
            }
            Event::Changed {
                chat_id,
                message,
                notify,
            } => {
                self.change_inflight = false;
                if !message.mine {
                    self.request_photo(&message.author_id);
                }
                // Splits delivery time into Microsoft+relay and our fetch. Uses the local clock
                // against Microsoft's timestamp, so it is only as accurate as clock sync.
                if let Some(pushed) = self.push_times.remove(&message.id)
                    && let Some(created) = model::parse_time(&message.created_at)
                {
                    let ms = |d: time::Duration| d.whole_milliseconds();
                    let now = time::OffsetDateTime::now_utc();
                    eprintln!(
                        "TeamsFast delivery: microsoft+relay={}ms fetch={}ms total={}ms",
                        ms(pushed - created),
                        ms(now - pushed),
                        ms(now - created)
                    );
                }
                let first = notify && self.notice_history.first(&chat_id, &message.id);
                let focused = self.focused;
                let reading = self.selected.as_ref() == Some(&chat_id) && focused;
                let at_bottom = self
                    .timelines
                    .get(&chat_id)
                    .is_some_and(|state| state.at_bottom);
                let muted = self.is_muted(&chat_id);
                let epoch = self.epoch();
                if !self.chats.iter().any(|chat| chat.summary.id == chat_id) {
                    self.chats.push(Chat::new(ChatSummary {
                        id: chat_id.clone(),
                        title: message.author.clone(),
                        ..Default::default()
                    }));
                    if !self.chats_loading {
                        self.refresh_chats(None, true);
                    }
                }
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.summary.id == chat_id)
                {
                    if notify && first && !message.mine && !message.deleted {
                        if !reading || !at_bottom {
                            chat.unread += 1;
                        }
                        if !reading
                            && !muted
                            && self.prefs.notifications
                            && !self.prefs.quiet
                            && let Some(notices) = &self.notices
                        {
                            let body = if self.prefs.notification_previews {
                                format!(
                                    "{}: {}",
                                    message.author,
                                    message.text.chars().take(180).collect::<String>()
                                )
                            } else {
                                "New message".into()
                            };
                            notices.show(epoch, chat_id.clone(), chat.summary.title.clone(), body);
                        }
                    }
                    if chat.pending.get(&message.id).is_some_and(|p| p.uncertain) {
                        chat.pending.remove(&message.id);
                        chat.revision = chat.revision.wrapping_add(1);
                    }
                    chat.merge_messages(vec![message]);
                    chat.recount_unread();
                    self.cache_dirty.insert(chat_id);
                }
                self.sort_chats();
            }
            Event::Deleted {
                chat_id,
                message_id,
            } => {
                self.change_inflight = false;
                if let Some(store) = &self.store
                    && let Err(error) = store.send(store::Command::Delete {
                        chat_id: chat_id.clone(),
                        message_id: message_id.clone(),
                    })
                {
                    self.error = Some(error);
                }
                self.cache_dirty.insert(chat_id.clone());
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.summary.id == chat_id)
                {
                    chat.revision = chat.revision.wrapping_add(1);
                    chat.message_versions
                        .insert(message_id.clone(), chat.revision);
                    if let Some(message) = chat
                        .messages
                        .iter_mut()
                        .find(|message| message.id == message_id)
                    {
                        message.delete();
                    }
                    if chat.summary.preview_id.as_ref() == Some(&message_id) {
                        chat.summary.preview = "Message deleted".into();
                    }
                    chat.pending.remove(&message_id);
                    chat.recount_unread();
                }
            }
            Event::Mutated {
                chat_id,
                message_id,
                operation_id,
                message,
            } => {
                if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id) {
                    if chat
                        .pending
                        .get(&message_id)
                        .is_some_and(|p| p.id == operation_id)
                    {
                        chat.pending.remove(&message_id);
                        chat.revision = chat.revision.wrapping_add(1);
                    }
                    chat.merge_messages(vec![message]);
                    chat.recount_unread();
                    self.cache_dirty.insert(chat_id);
                }
            }
            Event::Read { chat_id, marker } => {
                if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id) {
                    if model::timestamp(&marker)
                        >= chat
                            .summary
                            .read_at
                            .as_deref()
                            .map_or(i128::MIN, model::timestamp)
                    {
                        chat.summary.read_at = Some(marker.clone());
                    }
                    if chat.read_pending.as_ref() == Some(&marker) {
                        chat.read_pending = None;
                    }
                    chat.read_revision = self.revision;
                    chat.recount_unread();
                    self.cache_dirty.insert(chat_id);
                }
            }
            Event::Sent {
                chat_id,
                local_id,
                message,
            } => {
                self.notice_history.first(&chat_id, &message.id);
                if let Some(chat) = self
                    .chats
                    .iter_mut()
                    .find(|chat| chat.summary.id == chat_id)
                {
                    chat.complete_send(&local_id, message);
                    self.cache_removed
                        .entry(chat_id.clone())
                        .or_default()
                        .push(local_id);
                    self.cache_dirty.insert(chat_id.clone());
                    self.draft_dirty.insert(chat_id.clone());
                }
                self.timelines.entry(chat_id).or_default().jump_to_bottom = true;
                self.sort_chats();
                self.save_drafts();
            }
            Event::People { query, people } => {
                if let Some(dialog) = &mut self.new_chat {
                    dialog.loading = false;
                    if query == dialog.query.trim() {
                        dialog.people = people;
                        dialog.error = None;
                    }
                }
            }
            Event::Created(mut summary) => {
                if summary.title == "Untitled chat"
                    && let Some(dialog) = &self.new_chat
                {
                    summary.title = dialog
                        .selected
                        .iter()
                        .map(|person| person.display_name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                }
                if summary.updated_at.is_empty() {
                    summary.updated_at = model::now_string();
                }
                let id = summary.id.clone();
                if let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id) {
                    chat.summary = summary;
                } else {
                    self.chats.push(Chat::new(summary));
                }
                self.new_chat = None;
                self.search.clear();
                self.sort_chats();
                self.select_chat(id);
                self.refresh_chats(None, true);
            }
            Event::Photo { user_id, photo } => {
                let image = photo.and_then(|(mime, bytes)| {
                    ImageFormat::from_mime_type(&mime)
                        .map(|format| Arc::new(Image::from_bytes(format, bytes)))
                });
                self.photos.insert(user_id, image);
            }
            Event::Image { url, image } => {
                let image = image.map(|(image, size)| {
                    self.media_order.push_back((url.clone(), size));
                    self.media_bytes += size;
                    image
                });
                Arc::make_mut(&mut self.media).insert(url, image);
                self.evict_media();
                self.media_changed = true;
            }
            Event::Preview { url, preview } => {
                if let Some(image) = preview.as_ref().and_then(|preview| preview.image.clone()) {
                    if self.media_wanted.contains(&url) {
                        self.media_wanted.insert(image.clone());
                    }
                    self.request_media(image, false);
                }
                Arc::make_mut(&mut self.previews).insert(url, preview);
                self.media_changed = true;
            }
            Event::Watching { active, total } => {
                if crate::teams::tracing() {
                    eprintln!("TeamsFast watch: {active}/{total} subscriptions active");
                }
                self.watch_active = active;
                self.watch_total = total;
                self.watch_error = None;
            }
            Event::Warning(error) => self.error = Some(error),
            Event::Failed { operation, error } => {
                if let Some(delay) = error.retry_after {
                    self.retry_at = Instant::now() + delay;
                }
                match operation {
                    Operation::SignIn => {
                        // Microsoft's error text (e.g. an AADSTS code); no tokens.
                        if crate::teams::tracing() {
                            eprintln!("TeamsFast sign-in failed: {}", error.message);
                        }
                        if self.store.is_some() {
                            self.mode = Mode::Offline;
                        } else {
                            self.use_demo(false);
                        }
                        self.error = Some(error.message);
                    }
                    Operation::Chats => {
                        self.chats_loading = false;
                        self.error = Some(error.message);
                    }
                    Operation::Messages(id, kind) => {
                        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id)
                        {
                            chat.loading = false;
                        }
                        if kind == LoadKind::Catchup {
                            self.relay_status = error.message;
                        } else {
                            self.error = Some(error.message);
                        }
                    }
                    Operation::Message(id) => {
                        self.change_inflight = false;
                        if self.selected.as_ref() == Some(&id) {
                            self.error = Some(error.message);
                        } else {
                            self.relay_status = error.message;
                        }
                    }
                    Operation::Send(id, local_id) => {
                        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id)
                        {
                            if let Some(message) =
                                chat.messages.iter_mut().find(|m| m.id == local_id)
                            {
                                message.delivery = model::Delivery::Unconfirmed;
                                // Preserve the text for a deliberate resend; never retried here.
                                if chat.draft.trim().is_empty() {
                                    chat.draft = message.text.clone();
                                }
                                chat.revision = chat.revision.wrapping_add(1);
                            }
                            chat.send_error = Some(format!(
                                "Send not confirmed. Refresh before retrying. {}",
                                error.message
                            ));
                            self.draft_dirty.insert(id.clone());
                            self.cache_dirty.insert(id);
                        }
                    }
                    Operation::Mutate(chat_id, message_id, operation_id) => {
                        if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id)
                            && chat
                                .pending
                                .get(&message_id)
                                .is_some_and(|p| p.id == operation_id)
                        {
                            if error.uncertain {
                                chat.pending.get_mut(&message_id).unwrap().uncertain = true;
                                self.pending_changes
                                    .insert((chat_id, message_id), (false, false, None));
                            } else {
                                chat.pending.remove(&message_id);
                            }
                            chat.revision = chat.revision.wrapping_add(1);
                        }
                        self.error = Some(if error.uncertain {
                            format!("Update not confirmed. Refresh to check. {}", error.message)
                        } else {
                            error.message
                        });
                    }
                    Operation::MarkRead(chat_id, marker) => {
                        if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id)
                        {
                            if chat.read_pending.as_ref() == Some(&marker) {
                                chat.read_pending = None;
                            }
                            chat.recount_unread();
                        }
                        self.error = Some(error.message);
                    }
                    Operation::People(query) => {
                        if let Some(dialog) = &mut self.new_chat {
                            dialog.loading = false;
                            if dialog.query.trim() == query {
                                dialog.error = Some(error.message);
                            }
                        }
                    }
                    Operation::Create => {
                        if let Some(dialog) = &mut self.new_chat {
                            dialog.creating = false;
                            dialog.error = Some(format!(
                                "Chat creation was not confirmed. Refresh chats before retrying. {}",
                                error.message
                            ));
                        }
                    }
                    Operation::Watch => {
                        if crate::teams::tracing() {
                            eprintln!("TeamsFast watch error: {}", error.message);
                        }
                        self.watch_error = Some(error.message)
                    }
                }
            }
        }
    }

    fn request_media(&mut self, url: String, preview: bool) {
        let Some(worker) = &self.worker else {
            return;
        };
        if preview && !self.previews.contains_key(&url) {
            Arc::make_mut(&mut self.previews).insert(url.clone(), None);
            worker.media(Media::Preview(url));
        } else if !preview && !self.media.contains_key(&url) {
            Arc::make_mut(&mut self.media).insert(url.clone(), None);
            worker.media(Media::Image(url));
        }
    }

    /// Drops the oldest decoded images beyond `MEDIA_BUDGET`, keeping the open conversation's.
    /// Evicted URLs are fetched again when their conversation is reopened.
    fn evict_media(&mut self) {
        let mut kept = VecDeque::new();
        while self.media_bytes > MEDIA_BUDGET
            && let Some((url, size)) = self.media_order.pop_front()
        {
            if self.media_wanted.contains(&url) {
                kept.push_back((url, size));
                continue;
            }
            self.media_bytes -= size;
            if let Some(Some(image)) = Arc::make_mut(&mut self.media).remove(&url) {
                self.evicted.push(image);
            }
        }
        kept.append(&mut self.media_order);
        self.media_order = kept;
    }

    /// Requests images and link previews of the open conversation when it or its messages change.
    fn scan_media(&mut self) {
        let Some(chat) = self
            .selected
            .as_ref()
            .and_then(|id| self.chats.iter().find(|chat| &chat.summary.id == id))
        else {
            return;
        };
        let key = (chat.summary.id.clone(), chat.revision);
        if self.media_scanned.as_ref() == Some(&key) {
            return;
        }
        self.media_scanned = Some(key);
        let previews = !self.prefs.hide_link_previews;
        let wanted: Vec<_> = chat
            .messages
            .iter()
            .flat_map(|message| {
                let images = message.images.iter().map(|url| (url.clone(), false));
                let link = message
                    .link
                    .clone()
                    .filter(|_| previews)
                    .map(|url| (url, true));
                images.chain(link)
            })
            .collect();
        self.media_wanted = wanted
            .iter()
            .flat_map(|(url, preview)| {
                let image = preview
                    .then(|| self.previews.get(url).cloned().flatten()?.image)
                    .flatten();
                std::iter::once(url.clone()).chain(image)
            })
            .collect();
        for (url, preview) in wanted {
            self.request_media(url, preview);
        }
    }

    /// Adds or removes my `emoji` reaction, updating the timeline before Microsoft confirms.
    pub fn toggle_reaction(&mut self, chat_id: &str, message_id: &str, emoji: &str) {
        let Some(message) = self
            .chats
            .iter()
            .find(|c| c.summary.id == chat_id)
            .and_then(|c| c.messages.iter().find(|m| m.id == message_id))
        else {
            return;
        };
        let reaction = message.reactions.iter().find(|r| r.emoji == emoji);
        let change = MessageChange::Reaction {
            kind: reaction.map_or_else(|| emoji.to_owned(), |r| r.kind.clone()),
            emoji: emoji.into(),
            set: !reaction.is_some_and(|r| r.mine),
        };
        self.mutate_message(chat_id, message_id, change);
    }

    pub fn mutate_message(&mut self, chat_id: &str, message_id: &str, change: MessageChange) {
        if !matches!(self.mode, Mode::Live | Mode::Demo) {
            return;
        }
        let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id) else {
            return;
        };
        if chat.pending.contains_key(message_id) {
            return;
        }
        let Some(message) = chat
            .messages
            .iter_mut()
            .find(|m| m.id == message_id && !m.deleted && m.delivery == model::Delivery::Sent)
        else {
            return;
        };
        if !matches!(change, MessageChange::Reaction { .. }) && !message.mine {
            return;
        }
        // Text editing must not silently discard attachments, quotes or Teams formatting.
        if matches!(change, MessageChange::Edit(_))
            && (!message.files.is_empty() || message.quote.is_some() || !message.images.is_empty())
        {
            self.error = Some("Edit formatted messages and attachments in Teams; plain-text editing is supported here.".into());
            return;
        }
        if self.mode == Mode::Demo {
            change.apply(message);
            chat.revision = chat.revision.wrapping_add(1);
            return;
        }
        self.demo_message_id += 1;
        let operation_id = self.demo_message_id;
        chat.revision = chat.revision.wrapping_add(1);
        chat.pending.insert(
            message_id.into(),
            PendingChange {
                id: operation_id,
                revision: chat.revision,
                change: change.clone(),
                uncertain: false,
            },
        );
        if !self.request(Command::Mutate {
            chat_id: chat_id.into(),
            message_id: message_id.into(),
            operation_id,
            change,
        }) {
            self.chats
                .iter_mut()
                .find(|c| c.summary.id == chat_id)
                .unwrap()
                .pending
                .remove(message_id);
        }
    }

    fn open_store(&mut self) {
        self.store_ready = false;
        self.store = Some(Store::new(
            self.account.clone(),
            self.name.clone(),
            self.prefs
                .drafts
                .get(&self.account)
                .cloned()
                .unwrap_or_default(),
            self.wake.clone(),
        ));
    }

    fn receive_store(&mut self) {
        while let Some(event) = self.store.as_ref().and_then(|s| s.events.try_recv().ok()) {
            self.revision = self.revision.wrapping_add(1);
            match event {
                store::Event::Ready { name, chats } => {
                    self.store_ready = true;
                    self.prefs.drafts.remove(&self.account);
                    if self.mode != Mode::Live {
                        self.mode = Mode::Offline;
                        self.name = name;
                    }
                    for cached in chats {
                        let id = cached.summary.id.clone();
                        let index = self
                            .chats
                            .iter()
                            .position(|c| c.summary.id == id)
                            .unwrap_or_else(|| {
                                self.chats.push(Chat::new(cached.summary));
                                self.chats.len() - 1
                            });
                        let chat = &mut self.chats[index];
                        if !self.draft_dirty.contains(&id) {
                            chat.draft = cached.draft;
                        }
                        chat.merge_messages(cached.pending);
                        chat.recount_unread();
                    }
                    self.sort_chats();
                    if self.selected.is_none()
                        && let Some(id) = self
                            .chats
                            .iter()
                            .find(|c| !c.summary.is_self)
                            .or(self.chats.first())
                            .map(|c| c.summary.id.clone())
                    {
                        self.select_chat(id);
                    } else if let Some(id) = self.selected.clone() {
                        self.cached_history(&id);
                    }
                }
                store::Event::History {
                    chat_id,
                    revision,
                    mut messages,
                } => {
                    if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id) {
                        if revision < chat.history_cleared {
                            continue;
                        }
                        messages.retain(|m| {
                            m.delivery == model::Delivery::Sent
                                || !chat.messages.iter().any(|live| live.id == m.id)
                        });
                        chat.merge_messages_after(messages, Some(revision));
                        chat.recount_unread();
                    }
                }
                store::Event::Prepared {
                    chat_id,
                    message,
                    saved,
                } => {
                    let sent = saved
                        && self.mode == Mode::Live
                        && self.request(Command::Send {
                            chat_id: chat_id.clone(),
                            local_id: message.id.clone(),
                            text: message.text.clone(),
                        });
                    if !sent
                        && let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id)
                    {
                        if let Some(local) = chat.messages.iter_mut().find(|m| m.id == message.id) {
                            local.delivery = model::Delivery::Unconfirmed;
                        }
                        if chat.draft.is_empty() {
                            chat.draft = message.text;
                        }
                        chat.send_error = Some(if saved { "Message saved but not sent. Check your connection before retrying." } else { "Message was not sent: encrypted storage failed. Your text is still here." }.into());
                        chat.revision = chat.revision.wrapping_add(1);
                        self.draft_dirty.insert(chat_id.clone());
                        self.cache_dirty.insert(chat_id);
                    }
                }
                store::Event::Cleared => {}
                store::Event::Error(error) => {
                    self.error = Some(error);
                    // A failed write must not make an in-memory draft look clean.
                    self.draft_dirty
                        .extend(self.chats.iter().map(|c| c.summary.id.clone()));
                }
            }
        }
    }

    fn cached_history(&mut self, id: &str) {
        if let Some(store) = self.store.as_ref().filter(|_| self.store_ready) {
            let revision = self
                .chats
                .iter()
                .find(|c| c.summary.id == id)
                .map_or(0, |c| c.revision);
            if let Err(error) = store.send(store::Command::History {
                chat_id: id.into(),
                revision,
            }) {
                self.error = Some(error);
            }
        }
    }

    fn flush_cache(&mut self) {
        let Some(store) = self.store.as_ref().filter(|_| self.store_ready) else {
            return;
        };
        // Keep the queue bounded and leave room for durable drafts and send preparation.
        for id in self.cache_dirty.iter().take(8).cloned().collect::<Vec<_>>() {
            let Some(chat) = self.chats.iter().find(|c| c.summary.id == id) else {
                self.cache_dirty.remove(&id);
                continue;
            };
            if store
                .send(store::Command::Save {
                    summary: chat.summary.clone(),
                    messages: chat.messages.iter().rev().take(500).cloned().collect(),
                    remove: self.cache_removed.get(&id).cloned().unwrap_or_default(),
                })
                .is_err()
            {
                break;
            }
            self.cache_dirty.remove(&id);
            self.cache_removed.remove(&id);
        }
    }

    pub fn discard_local_send(&mut self, chat_id: &str, message_id: &str) {
        if let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == chat_id)
            && chat
                .messages
                .iter()
                .any(|m| m.id == message_id && m.delivery == model::Delivery::Unconfirmed)
        {
            chat.remove_message(message_id);
            self.cache_removed
                .entry(chat_id.into())
                .or_default()
                .push(message_id.into());
            self.cache_dirty.insert(chat_id.into());
        }
    }

    pub fn clear_history(&mut self) {
        if self
            .chats
            .iter()
            .any(|c| !c.pending.is_empty() || c.loading)
        {
            self.error =
                Some("Wait for current message updates to finish before clearing history.".into());
            return;
        }
        let Some(store) = &self.store else { return };
        if let Err(error) = store.send(store::Command::ClearHistory) {
            self.error = Some(error);
            return;
        }
        self.cache_dirty.clear();
        for chat in &mut self.chats {
            for message in &chat.messages {
                if message.delivery == model::Delivery::Sent {
                    chat.message_versions
                        .insert(message.id.clone(), chat.revision.wrapping_add(1));
                }
            }
            chat.messages
                .retain(|m| m.delivery != model::Delivery::Sent);
            chat.summary.preview.clear();
            chat.summary.preview_id = None;
            chat.loaded = false;
            chat.revision = chat.revision.wrapping_add(1);
            chat.history_cleared = chat.revision;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    fn cancel_read(&mut self) {
        if let Some((_, active)) = self.read_active.take() {
            active.store(false, Ordering::Relaxed);
        }
    }

    pub fn mark_read(&mut self, id: &str, reading: bool) -> bool {
        if !reading {
            self.cancel_read();
            return false;
        }
        if self.mode != Mode::Live {
            return false;
        }
        let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == id) else {
            return false;
        };
        if chat.loading || !chat.loaded || chat.summary.is_self {
            return false;
        }
        let Some(marker) = chat
            .messages
            .iter()
            .filter(|m| m.delivery == model::Delivery::Sent)
            .map(|m| &m.created_at)
            .max_by_key(|t| model::timestamp(t))
            .cloned()
        else {
            return false;
        };
        if chat.read_pending.is_some()
            || chat.read_attempted.as_ref() == Some(&marker)
            || chat
                .summary
                .read_at
                .as_ref()
                .is_some_and(|read| model::timestamp(read) >= model::timestamp(&marker))
        {
            return false;
        }
        chat.read_attempted = Some(marker.clone());
        chat.read_pending = Some(marker.clone());
        chat.recount_unread();
        let active = Arc::new(AtomicBool::new(true));
        self.cancel_read();
        self.read_active = Some((id.into(), active.clone()));
        if !self.request(Command::MarkRead {
            chat_id: id.into(),
            marker,
            active,
            deadline: Instant::now() + Duration::from_secs(1),
        }) {
            let chat = self.chats.iter_mut().find(|c| c.summary.id == id).unwrap();
            chat.read_pending = None;
            chat.recount_unread();
        }
        true
    }

    pub fn tick(&mut self) {
        if !self.focused {
            self.cancel_read();
        }
        self.receive();
        self.receive_store();
        self.save_drafts();
        self.flush_cache();
        if self.store_ready {
            for chat in &mut self.chats {
                if self.hot_chats.contains(&chat.summary.id)
                    || self.cache_dirty.contains(&chat.summary.id)
                    || chat.messages.len() <= 1
                {
                    continue;
                }
                let before = chat.messages.len();
                let latest = chat.messages.last().map(|m| m.id.clone());
                chat.messages.retain(|m| {
                    Some(&m.id) == latest.as_ref()
                        || m.delivery != model::Delivery::Sent
                        || chat.pending.contains_key(&m.id)
                });
                if chat.messages.len() != before {
                    chat.loaded = false;
                    chat.revision = chat.revision.wrapping_add(1);
                }
            }
        }
        self.scan_media();
        let unread = self
            .chats
            .iter()
            .filter(|chat| !self.is_muted(&chat.summary.id))
            .map(|chat| chat.unread)
            .sum();
        if unread != self.badge {
            self.badge = unread;
            crate::notifications::set_badge(unread);
        }
        if self.mode != Mode::Live {
            return;
        }
        if Instant::now() < self.retry_at {
            return;
        }
        if let Some(registration) = self.watch_pending.take()
            && let Some(worker) = &self.worker
            && worker
                .send(Command::Watch(Some(registration.clone())))
                .is_err()
        {
            self.watch_pending = Some(registration);
        }
        let next_change = if self.change_inflight {
            None
        } else {
            let selected = self
                .pending_changes
                .keys()
                .find(|(chat, _)| Some(chat) == self.selected.as_ref())
                .cloned();
            selected
                .and_then(|key| self.pending_changes.remove(&key).map(|value| (key, value)))
                .or_else(|| self.pending_changes.pop_first())
        };
        if let Some(((chat_id, message_id), (notify, deleted, rich))) = next_change {
            if self.request(Command::Message {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                notify,
                deleted,
                rich,
            }) {
                self.change_inflight = true;
            } else {
                self.pending_changes
                    .insert((chat_id, message_id), (notify, deleted, None));
            }
        }
        let query = self
            .new_chat
            .as_ref()
            .filter(|dialog| {
                !dialog.loading
                    && dialog.search_at.is_some_and(|at| Instant::now() >= at)
                    && dialog.query.trim().chars().count() >= 2
            })
            .map(|dialog| dialog.query.trim().to_owned());
        if let Some(query) = query
            && self.request(Command::People { query })
            && let Some(dialog) = &mut self.new_chat
        {
            dialog.loading = true;
            dialog.search_at = None;
        }
    }

    pub fn sort_chats(&mut self) {
        self.chats.sort_by_cached_key(|chat| {
            std::cmp::Reverse((
                chat.summary.is_self,
                model::timestamp(&chat.summary.updated_at),
            ))
        });
    }
    pub fn refresh_chats(&mut self, next: Option<String>, background: bool) {
        if self.request(Command::Chats {
            next,
            background,
            started_revision: self.revision,
        }) {
            self.chats_loading = true;
        }
    }
    pub fn select_chat(&mut self, id: String) {
        if !self.chats.iter().any(|chat| chat.summary.id == id) {
            return;
        }
        let recent = self
            .prefs
            .recent_chats
            .entry(self.account.clone())
            .or_default();
        recent.retain(|chat| chat != &id);
        recent.insert(0, id.clone());
        recent.truncate(20);
        self.cancel_read();
        self.hot_chats.retain(|chat| chat != &id);
        self.hot_chats.push_front(id.clone());
        self.hot_chats.truncate(3);
        self.selected = Some(id.clone());
        self.timelines
            .entry(id.clone())
            .or_insert_with(|| Timeline {
                jump_to_bottom: true,
                ..Default::default()
            });
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id) {
            chat.read_attempted = None;
        }
        self.cached_history(&id);
        if self.mode == Mode::Live {
            let kind = if self
                .chats
                .iter()
                .any(|chat| chat.summary.id == id && chat.loaded)
            {
                LoadKind::Refresh
            } else {
                LoadKind::Initial
            };
            self.refresh_messages(id, None, kind);
        }
    }
    pub fn refresh_messages(&mut self, id: String, next: Option<String>, kind: LoadKind) {
        if self
            .chats
            .iter()
            .any(|chat| chat.summary.id == id && chat.loading)
        {
            return;
        }
        if matches!(kind, LoadKind::Refresh | LoadKind::Initial)
            && let Some(chat) = self.chats.iter_mut().find(|c| c.summary.id == id)
        {
            chat.read_attempted = None;
        }
        if self.request(Command::Messages {
            chat_id: id.clone(),
            next,
            kind,
            started_revision: self
                .chats
                .iter()
                .find(|chat| chat.summary.id == id)
                .map_or(0, |chat| chat.revision),
        }) && let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id)
        {
            chat.loading = true;
        }
    }
    pub fn load_older(&mut self, id: String) {
        let next = self
            .chats
            .iter()
            .find(|chat| chat.summary.id == id)
            .and_then(|chat| chat.next_messages.clone());
        if self.mode == Mode::Demo {
            if let Some(chat) = self.chats.iter_mut().find(|chat| chat.summary.id == id) {
                self.timelines.entry(id).or_default().preserve_anchor = true;
                let when = time::OffsetDateTime::now_utc() - time::Duration::days(1);
                chat.merge_messages(vec![Message{id:"demo-older-message".into(),author:"Maya Chen".into(),author_id:"Maya Chen".into(),
                    text:"This earlier message was loaded without replacing the current history or draft.".into(),
                    created_at:when.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),..Default::default()}]);
                chat.next_messages = None;
            }
        } else if next.is_some() {
            self.refresh_messages(id, next, LoadKind::Older);
        }
    }

    /// Shows the message immediately and clears the composer; Microsoft's confirmation replaces
    /// the placeholder. Failures mark it unconfirmed and restore the text. Never retried.
    pub fn send_message(&mut self) {
        let Some(index) = self
            .chats
            .iter()
            .position(|chat| Some(&chat.summary.id) == self.selected.as_ref())
        else {
            return;
        };
        let chat = &self.chats[index];
        if chat.draft.trim().is_empty() || !matches!(self.mode, Mode::Live | Mode::Demo) {
            return;
        }
        let text = chat.draft.clone();
        let chat_id = chat.summary.id.clone();
        self.demo_message_id += 1;
        let live = self.mode == Mode::Live;
        let id = format!(
            "{}-{}-{}",
            if live { "local" } else { "demo-sent" },
            time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
            self.demo_message_id
        );
        let message = Message {
            id,
            author: if live {
                self.name.clone()
            } else {
                "You".into()
            },
            author_id: if live {
                self.user_id.clone()
            } else {
                "demo-self".into()
            },
            text,
            created_at: model::now_string(),
            mine: true,
            delivery: if live {
                model::Delivery::Sending
            } else {
                model::Delivery::Sent
            },
            ..Default::default()
        };
        if live {
            let result = self
                .store
                .as_ref()
                .filter(|_| self.store_ready)
                .ok_or_else(|| {
                    "Encrypted history is not ready. Your draft has not been sent.".to_owned()
                })
                .and_then(|s| {
                    s.send(store::Command::Prepare {
                        summary: self.chats[index].summary.clone(),
                        message: Box::new(message.clone()),
                    })
                });
            if let Err(error) = result {
                self.error = Some(error);
                return;
            }
        }
        let chat = &mut self.chats[index];
        chat.merge_messages(vec![message]);
        chat.draft.clear();
        chat.send_error = None;
        self.timelines.entry(chat_id).or_default().jump_to_bottom = true;
        self.sort_chats();
        self.save_drafts();
    }
}
