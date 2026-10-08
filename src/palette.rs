//! Keyboard navigation built from Kit's Command, Dialog and Kbd components.
use crate::{
    app::{
        NewConversation, OpenSettings, Refresh, SearchChats, ShowCommands, ShowShortcuts,
        SwitchConversation, TeamsFast, ToggleAppearance, ToggleMute,
    },
    state::Mode,
};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, WindowExt,
        badge::Badge,
        command::{Command, CommandItem, CommandState},
        h_flex,
        kbd::Kbd,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{rc::Rc, sync::Arc};

/// Shared by the app and dialog content: Actions also work while the palette input owns focus.
pub(crate) fn action_scope(view: WeakEntity<TeamsFast>) -> Div {
    div()
        .key_context("TeamsFast")
        .on_action(handle::<SwitchConversation>(
            &view,
            TeamsFast::open_switcher,
        ))
        .on_action(handle::<ShowCommands>(&view, TeamsFast::open_commands))
        .on_action(handle::<ShowShortcuts>(&view, TeamsFast::open_shortcuts))
        .on_action(handle::<OpenSettings>(&view, TeamsFast::open_settings))
        .on_action(handle::<NewConversation>(&view, TeamsFast::open_new_chat))
        .on_action(handle::<Refresh>(&view, TeamsFast::refresh))
        .on_action(handle::<SearchChats>(&view, |this, window, cx| {
            this.search.focus_handle(cx).focus(window, cx)
        }))
        .on_action(handle::<ToggleMute>(&view, |this, _, cx| {
            if let Some(id) = this.state.selected.clone() {
                this.state.toggle_mute(id);
                this.changed(cx);
            }
        }))
        .on_action(handle::<ToggleAppearance>(&view, |this, window, cx| {
            this.state.prefs.light_theme = !this.state.prefs.light_theme;
            crate::ui::apply_theme(this.state.prefs.light_theme, Some(window), cx);
            this.changed(cx);
        }))
}

fn handle<A: Action>(
    view: &WeakEntity<TeamsFast>,
    action: fn(&mut TeamsFast, &mut Window, &mut Context<TeamsFast>),
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let view = view.clone();
    move |_, window, cx| {
        // Close the old palette before dispatching an action that might open another dialog.
        window.close_dialog(cx);
        let _ = view.update(cx, |this, cx| {
            this.dialog = None;
            action(this, window, cx);
        });
    }
}

struct Choice {
    id: String,
    title: String,
    photo: Option<Arc<Image>>,
    unread: usize,
    current: bool,
    listed: bool,
}

impl TeamsFast {
    pub(crate) fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Snapshot the ordering while open: an incoming message must not move the item under Return.
        let recent = self.state.prefs.recent_chats.get(&self.state.account);
        let mut choices: Vec<_> = self
            .state
            .chats
            .iter()
            .filter(|chat| chat.visible())
            .enumerate()
            .map(|(index, chat)| {
                let id = &chat.summary.id;
                let current = self.state.selected.as_ref() == Some(id);
                let recent_index = recent
                    .and_then(|ids| ids.iter().position(|r| r == id))
                    .unwrap_or(usize::MAX);
                (
                    (current, recent_index, index),
                    Choice {
                        id: id.clone(),
                        title: chat.summary.title.clone(),
                        photo: self.state.photo(chat.summary.avatar_user.as_deref()),
                        unread: chat.unread,
                        current,
                        listed: chat.listed(false),
                    },
                )
            })
            .collect();
        choices.sort_by_key(|(rank, _)| *rank);
        let choices: Rc<Vec<Choice>> =
            Rc::new(choices.into_iter().map(|(_, choice)| choice).collect());
        let state = cx.new(|cx| CommandState::new(window, cx));
        let focus = state.clone();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let query = state.read(cx).query(cx).trim().to_lowercase();
            let mut matches: Vec<_> = choices
                .iter()
                .enumerate()
                .filter(|(_, c)| !query.is_empty() || c.listed)
                .filter_map(|(i, c)| fuzzy_rank(&c.title, &query).map(|score| (score, i)))
                .collect();
            matches.sort_by_key(|(score, i)| (*score, *i));
            let ids: Vec<_> = matches
                .iter()
                .map(|(_, i)| choices[*i].id.clone())
                .collect();
            let items = matches.into_iter().map(|(_, i)| {
                let c = &choices[i];
                let (title, photo, unread, current) =
                    (c.title.clone(), c.photo.clone(), c.unread, c.current);
                let label = format!(
                    "{}{}{}",
                    title,
                    if current { " · current" } else { "" },
                    if unread > 0 {
                        format!(" · {unread} unread")
                    } else {
                        String::new()
                    }
                );
                CommandItem::new().label(label).child(move |_, cx| {
                    h_flex()
                        .h(px(32.))
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .text_size(px(13.))
                        .child(div().flex_shrink_0().child(
                            Badge::new().count(unread).child(crate::ui::avatar(
                                &title,
                                photo.clone(),
                                px(28.),
                            )),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(if unread > 0 {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .child(title.clone()),
                        )
                        .when(current, |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Current"),
                            )
                        })
                })
            });
            let confirmed = view.clone();
            let palette = Command::new(&state)
                .items(items)
                .filterable(false)
                .bordered(false)
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .placeholder("Search all conversations…")
                .max_h(px(280.))
                .empty(|_, _, cx| {
                    div()
                        .px_3()
                        .py_6()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No matching conversations. Try another name.")
                })
                .on_query(|_, window, _| window.refresh())
                .on_confirm(move |index, window, cx| {
                    if let Some(id) = ids.get(index.row) {
                        window.close_dialog(cx);
                        let _ = confirmed.update(cx, |this, cx| {
                            this.state.search.clear();
                            this.select_chat(id.clone(), window, cx);
                        });
                    }
                })
                .footer(|_, _, cx| palette_keys(cx));
            dialog
                .title("Jump to conversation")
                .width(px(520.))
                .margin_top(px(48.))
                .child(action_scope(view.clone()).child(palette))
        });
        focus.update(cx, |state, cx| state.focus(window, cx));
    }

    pub(crate) fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let focus = state.clone();
        let view = cx.entity().downgrade();
        let can_create = matches!(self.state.mode, Mode::Live | Mode::Demo);
        let live = self.state.mode == Mode::Live;
        let selected = self.state.selected.clone();
        let muted = selected.as_ref().is_some_and(|id| self.state.is_muted(id));
        let light = self.state.prefs.light_theme;
        window.open_dialog(cx, move |dialog, _, cx| {
            let palette = Command::new(&state)
                .bordered(false)
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .placeholder("Type a command…")
                .max_h(px(280.))
                .item(
                    CommandItem::new()
                        .label("Jump to conversation")
                        .action(Box::new(SwitchConversation)),
                )
                .item(
                    CommandItem::new()
                        .label("New conversation")
                        .disabled(!can_create)
                        .action(Box::new(NewConversation)),
                )
                .item(
                    CommandItem::new()
                        .label("Refresh chats and messages")
                        .disabled(!live)
                        .action(Box::new(Refresh)),
                )
                .item(
                    CommandItem::new()
                        .label(if muted {
                            "Unmute conversation"
                        } else {
                            "Mute conversation"
                        })
                        .disabled(selected.is_none())
                        .action(Box::new(ToggleMute)),
                )
                .item(
                    CommandItem::new()
                        .label(if light {
                            "Use dark appearance"
                        } else {
                            "Use light appearance"
                        })
                        .action(Box::new(ToggleAppearance)),
                )
                .item(
                    CommandItem::new()
                        .label("Settings")
                        .action(Box::new(OpenSettings)),
                )
                .item(
                    CommandItem::new()
                        .label("Check for updates")
                        .action(Box::new(crate::app::CheckForUpdates)),
                )
                .item(
                    CommandItem::new()
                        .label("Keyboard shortcuts")
                        .action(Box::new(ShowShortcuts)),
                )
                .footer(|_, _, cx| palette_keys(cx));
            // Command dispatches the Action first. action_scope closes this dialog before opening
            // a replacement; an on_confirm close here would accidentally close that replacement.
            dialog
                .title("Commands")
                .width(px(520.))
                .margin_top(px(48.))
                .child(action_scope(view.clone()).child(palette))
        });
        focus.update(cx, |state, cx| state.focus(window, cx));
    }

    pub(crate) fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, _| {
            let bindings: Vec<(&str, Box<dyn Action>)> = vec![
                ("Jump to conversation", Box::new(SwitchConversation)),
                ("App commands", Box::new(ShowCommands)),
                ("Keyboard shortcuts", Box::new(ShowShortcuts)),
                ("Filter conversation list", Box::new(SearchChats)),
                ("New conversation", Box::new(NewConversation)),
                ("Refresh", Box::new(Refresh)),
                ("Settings", Box::new(OpenSettings)),
            ];
            let rows = bindings.into_iter().map(|(label, action)| {
                h_flex().justify_between().gap_4().child(label).when_some(
                    Kbd::binding_for_action(&*action, Some("TeamsFast"), window),
                    |row, kbd| row.child(kbd),
                )
            });
            dialog.title("Keyboard shortcuts").width(px(440.)).child(
                action_scope(view.clone()).child(
                    v_flex()
                        .gap_3()
                        .children(rows)
                        .child(
                            h_flex()
                                .justify_between()
                                .child("Send message")
                                .child(key("enter")),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .child("New line")
                                .child(key("shift-enter")),
                        ),
                ),
            )
        });
    }
}

fn palette_keys(cx: &App) -> Div {
    h_flex()
        .justify_between()
        .px_3()
        .py_2()
        .gap_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            h_flex()
                .gap_1()
                .child(key("up"))
                .child(key("down"))
                .child(div().ml_1().child("Navigate")),
        )
        .child(h_flex().gap_2().child(key("enter")).child("Open"))
        .child(h_flex().gap_2().child(key("escape")).child("Clear / close"))
}

fn key(stroke: &str) -> Kbd {
    Kbd::new(Keystroke::parse(stroke).expect("static shortcut"))
}

/// Prefer exact, prefix and substring matches, then compact Unicode subsequences. No dependency
/// or network query is needed for the small in-memory conversation list.
fn fuzzy_rank(title: &str, query: &str) -> Option<(u8, usize)> {
    let title = title.to_lowercase();
    if query.is_empty() || title == query {
        return Some((0, 0));
    }
    if title.starts_with(query) {
        return Some((1, 0));
    }
    if let Some(index) = title.find(query) {
        return Some((2, index));
    }
    let mut letters = title.chars().enumerate();
    let mut first = None;
    let mut last = 0;
    let mut count = 0;
    for letter in query.chars().filter(|c| !c.is_whitespace()) {
        let (index, _) = letters.find(|(_, c)| *c == letter)?;
        first.get_or_insert(index);
        last = index;
        count += 1;
    }
    Some((3, last + 1 - first.unwrap_or(0) - count))
}
