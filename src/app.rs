use crate::{
    model::{self, Chat, ChatSummary, Message, Person},
    state::{ChatState, Mode},
    teams::{Command, LoadKind},
};
use gpui_kit::{
    component::{
        input::{InputEvent, InputState, TextareaState},
        message_scroller::MessageScrollerState,
    },
    *,
};
use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

gpui_kit::actions!(
    teamsfast,
    [Quit, OpenSettings, SearchChats, NewConversation, Refresh]
);

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DialogKind {
    NewChat,
    Disconnect,
}

pub(crate) struct Transcript {
    pub scroll: Entity<MessageScrollerState>,
    pub messages: Rc<Vec<Message>>,
    pub revision: u64,
}

pub struct TeamsFast {
    pub(crate) state: ChatState,
    /// A press in a title area that becomes a window drag once the mouse moves.
    pub(crate) title_drag: bool,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) search: Entity<InputState>,
    pub(crate) client_id: Entity<InputState>,
    pub(crate) tenant: Entity<InputState>,
    pub(crate) relay_url: Entity<InputState>,
    pub(crate) people_query: Entity<InputState>,
    pub(crate) topic: Entity<InputState>,
    pub(crate) transcripts: HashMap<String, Transcript>,
    pub(crate) dialog: Option<DialogKind>,
    pub(crate) settings_window: Option<AnyWindowHandle>,
    pub(crate) main_window: AnyWindowHandle,
    composer_chat: Option<String>,
    transcript_account: String,
    dirty: bool,
    last_save: Instant,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl TeamsFast {
    pub fn new(demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake, receiver) = async_channel::bounded(1);
        let state = ChatState::new(demo, wake);
        crate::ui::apply_theme(state.prefs.light_theme, Some(window), cx);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write a message…")
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search conversations"));
        let client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.prefs.client_id.clone())
                .placeholder("Application (client) ID")
        });
        let tenant = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.prefs.tenant.clone())
                .placeholder("Tenant ID or domain")
        });
        let relay_url = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.prefs.relay_url.clone())
                .placeholder("https://relay.example.com")
        });
        let people_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search name or email"));
        let topic = cx.new(|cx| InputState::new(window, cx).placeholder("Group name (optional)"));
        let subscriptions = vec![
            cx.subscribe_in(
                &composer,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        if let Some(id) = &this.state.selected
                            && let Some(chat) = this
                                .state
                                .chats
                                .iter_mut()
                                .find(|chat| &chat.summary.id == id)
                        {
                            chat.draft = input.read(cx).value().to_string();
                            this.dirty = true;
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift, .. } if !shift => this.send(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe(&search, |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.state.search = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&people_query, |this, input, event, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(dialog) = &mut this.state.new_chat
                {
                    dialog.query = input.read(cx).value().to_string();
                    dialog.search_at = Some(Instant::now() + Duration::from_millis(300));
                    if this.state.mode == Mode::Demo {
                        let query = dialog.query.to_lowercase();
                        dialog.people = ["Maya Chen", "Alex Morgan", "Jordan Lee", "Sam Rivera"]
                            .into_iter()
                            .filter(|name| name.to_lowercase().contains(&query))
                            .map(|name| Person {
                                id: name.into(),
                                display_name: name.into(),
                                mail: None,
                                user_principal_name: None,
                            })
                            .collect();
                    }
                    cx.notify();
                }
            }),
        ];
        let wake_task = cx.spawn_in(window, async move |this, cx| {
            while receiver.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| this.drain_events(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let timer = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let read_changed = this.mark_read(window, cx);
                        let before = this.state.revision;
                        this.state.tick();
                        if read_changed || before != this.state.revision {
                            this.synchronize(window, cx);
                            cx.notify();
                        }
                        if this.dirty && this.last_save.elapsed() >= Duration::from_secs(2) {
                            this.persist();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut app = Self {
            state,
            title_drag: false,
            composer,
            search,
            client_id,
            tenant,
            relay_url,
            people_query,
            topic,
            transcripts: HashMap::new(),
            dialog: None,
            settings_window: None,
            main_window: window.window_handle(),
            composer_chat: None,
            transcript_account: String::new(),
            dirty: false,
            last_save: Instant::now(),
            _subscriptions: subscriptions,
            _tasks: vec![wake_task, timer],
        };
        app.synchronize(window, cx);
        app
    }

    pub(crate) fn drain_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_read(window, cx);
        self.state.tick();
        self.synchronize(window, cx);
        self.dirty = true;
        cx.notify();
    }

    fn mark_read(&mut self, window: &Window, cx: &App) -> bool {
        self.state.focused = window.is_window_active();
        if let Some(id) = &self.state.selected
            && let Some(transcript) = self.transcripts.get(id)
        {
            let at_bottom = !transcript.scroll.read(cx).is_scrolled_up();
            self.state
                .timelines
                .entry(id.clone())
                .or_default()
                .at_bottom = at_bottom;
            if at_bottom
                && self.state.focused
                && let Some(chat) = self
                    .state
                    .chats
                    .iter_mut()
                    .find(|chat| &chat.summary.id == id)
                && chat.unread > 0
            {
                chat.unread = 0;
                return true;
            }
        }
        false
    }

    pub(crate) fn synchronize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.transcript_account != self.state.account {
            self.transcripts.clear();
            self.transcript_account.clone_from(&self.state.account);
            self.composer_chat = None;
        }
        for chat in &self.state.chats {
            if !chat.loaded && chat.messages.is_empty() {
                continue;
            }
            let id = chat.summary.id.clone();
            let next = &chat.messages;
            let transcript = self
                .transcripts
                .entry(id.clone())
                .or_insert_with(|| Transcript {
                    scroll: cx.new(|cx| MessageScrollerState::new(0, cx)),
                    messages: Rc::new(Vec::new()),
                    revision: u64::MAX,
                });
            if transcript.revision != chat.revision {
                let old = &transcript.messages;
                let prefix = old
                    .iter()
                    .zip(next)
                    .take_while(|(a, b)| a.id == b.id)
                    .count();
                let suffix = old[prefix..]
                    .iter()
                    .rev()
                    .zip(next[prefix..].iter().rev())
                    .take_while(|(a, b)| a.id == b.id)
                    .count();
                if prefix != old.len() || prefix != next.len() {
                    transcript.scroll.update(cx, |scroll, cx| {
                        scroll.splice(prefix..old.len() - suffix, next.len() - prefix - suffix, cx);
                        // The row before the splice shows its time only as the last of a group.
                        if prefix > 0 {
                            scroll.remeasure_items(prefix - 1..prefix, cx);
                        }
                        if suffix > 0 {
                            let boundary = next.len() - suffix;
                            scroll.remeasure_items(boundary..boundary + 1, cx);
                        }
                    });
                }
                let retained = old[..prefix].iter().zip(&next[..prefix]).enumerate().chain(
                    old[old.len() - suffix..]
                        .iter()
                        .zip(&next[next.len() - suffix..])
                        .enumerate()
                        .map(|(index, pair)| (next.len() - suffix + index, pair)),
                );
                for (index, (before, after)) in retained {
                    if before != after {
                        transcript.scroll.update(cx, |scroll, cx| {
                            scroll.remeasure_items(index..(index + 2).min(next.len()), cx);
                        });
                    }
                }
                transcript.messages = Rc::new(next.clone());
                transcript.revision = chat.revision;
            }
            if self
                .state
                .timelines
                .get_mut(&id)
                .is_some_and(|state| std::mem::take(&mut state.jump_to_bottom))
            {
                transcript
                    .scroll
                    .update(cx, |scroll, cx| scroll.scroll_to_end(cx));
            }
        }
        for image in self.state.evicted.drain(..) {
            cx.drop_image(image, Some(window));
        }
        // Loaded images and link previews change row heights.
        if std::mem::take(&mut self.state.media_changed) {
            for transcript in self.transcripts.values() {
                let rows: Vec<_> = (0..transcript.messages.len())
                    .filter(|&index| {
                        let message = &transcript.messages[index];
                        !message.images.is_empty() || message.link.is_some()
                    })
                    .collect();
                if !rows.is_empty() {
                    transcript.scroll.update(cx, |scroll, cx| {
                        let at_bottom = !scroll.is_scrolled_up();
                        for index in rows {
                            scroll.remeasure_items(index..index + 1, cx);
                        }
                        // Taller rows must not pull a reader away from the newest message.
                        if at_bottom {
                            scroll.scroll_to_end(cx);
                        }
                    });
                }
            }
        }
        if self.search.read(cx).value().as_ref() != self.state.search {
            self.search.update(cx, |input, cx| {
                input.set_value(self.state.search.clone(), window, cx)
            });
        }
        let draft = self
            .state
            .selected
            .as_ref()
            .and_then(|id| self.state.chats.iter().find(|chat| &chat.summary.id == id))
            .map(|chat| chat.draft.clone())
            .unwrap_or_default();
        if self.composer_chat != self.state.selected
            || self.composer.read(cx).value().as_ref() != draft
        {
            self.composer
                .update(cx, |input, cx| input.set_value(draft, window, cx));
            self.composer_chat = self.state.selected.clone();
        }
        if self.dialog.is_some_and(|kind| match kind {
            DialogKind::NewChat => self.state.new_chat.is_none(),
            DialogKind::Disconnect => !self.state.disconnect_open,
        }) {
            window.close_dialog(cx);
            self.dialog = None;
        }
        if self.state.focus_requested {
            self.state.focus_requested = false;
            window.activate_window();
            self.composer.focus_handle(cx).focus(window, cx);
        }
    }

    pub(crate) fn select_chat(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state.select_chat(id);
        self.synchronize(window, cx);
        self.composer.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(crate) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.send_message();
        self.synchronize(window, cx);
        self.dirty = true;
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.mode == Mode::Live {
            self.state.refresh_chats(None, false);
            if let Some(id) = self.state.selected.clone() {
                self.state.refresh_messages(id, None, LoadKind::Refresh);
            }
        }
        self.synchronize(window, cx);
        cx.notify();
    }

    pub(crate) fn start_sign_in(
        &mut self,
        use_code: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.prefs.client_id = self.client_id.read(cx).value().trim().to_string();
        self.state.prefs.tenant = self.tenant.read(cx).value().trim().to_string();
        self.state.start_sign_in(use_code);
        self.dirty = true;
        self.synchronize(window, cx);
        cx.notify();
    }

    pub(crate) fn connect_relay(&mut self, cx: &mut Context<Self>) {
        self.state.prefs.relay_url = self.relay_url.read(cx).value().to_string();
        self.state.start_push();
        self.dirty = true;
        cx.notify();
    }

    pub(crate) fn create_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.state.new_chat else {
            return;
        };
        if dialog.selected.is_empty() || dialog.creating {
            return;
        }
        let selected = dialog.selected.clone();
        let topic = self.topic.read(cx).value().to_string();
        if self.state.mode == Mode::Demo {
            self.state.demo_message_id += 1;
            let id = format!("demo-created-{}", self.state.demo_message_id);
            let mut chat = Chat::new(ChatSummary {
                id: id.clone(),
                title: if topic.trim().is_empty() {
                    selected
                        .iter()
                        .map(|p| p.display_name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    topic
                },
                members: selected.len() + 1,
                updated_at: model::now_string(),
                ..Default::default()
            });
            chat.loaded = true;
            self.state.chats.insert(0, chat);
            self.state.new_chat = None;
            self.state.search.clear();
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.state.select_chat(id);
        } else if self.state.request(Command::Create {
            people: selected.iter().map(|p| p.id.clone()).collect(),
            topic: topic.clone(),
        }) && let Some(dialog) = &mut self.state.new_chat
        {
            dialog.topic = topic;
            dialog.creating = true;
            dialog.error = None;
        }
        self.synchronize(window, cx);
        cx.notify();
    }

    pub(crate) fn persist(&mut self) {
        self.last_save = Instant::now();
        if self.state.force_demo || !self.state.can_save_settings {
            self.dirty = false;
            return;
        }
        self.state.save_drafts();
        match self.state.prefs.save() {
            Ok(()) => self.dirty = false,
            Err(error) => self.state.error = Some(error),
        }
    }

    pub(crate) fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        cx.notify();
    }

    pub(crate) fn status(&self) -> (String, bool) {
        if self.state.mode == Mode::Demo {
            return ("Local demo".into(), false);
        }
        if self.state.mode == Mode::SigningIn {
            return ("Signing in…".into(), false);
        }
        if self.state.relay_connected
            && self.state.watch_active > 0
            && self.state.watch_active < self.state.watch_total
        {
            return (
                format!(
                    "Live updates · {}/{}",
                    self.state.watch_active, self.state.watch_total
                ),
                false,
            );
        }
        if self.state.watch_error.is_some() {
            return ("Live updates need attention".into(), false);
        }
        if self.state.relay_connected
            && self.state.watch_total > 0
            && self.state.watch_active >= self.state.watch_total
        {
            return ("Live updates".into(), true);
        }
        if self.state.relay_connected {
            return ("Connecting live updates…".into(), false);
        }
        (self.state.relay_status.clone(), false)
    }
}

impl Drop for TeamsFast {
    fn drop(&mut self) {
        self.persist();
    }
}

use gpui_kit::component::WindowExt as _;
