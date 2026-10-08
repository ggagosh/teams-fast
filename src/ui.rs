//! THESIS: a compact native messenger built from GPUI Kit controls.
//! OWN-WORLD: Kit's semantic dark/light themes, system type, Lucide icons.
//! STORY: find a conversation, read, reply, and return to work.
//! FIRST VIEWPORT: resizable conversation list, virtualized transcript, bottom composer.
//! FORM: user-selected GPUI Kit conventions, with GPUI Fast retained rendering.
use crate::{
    app::{NewConversation, OpenSettings, Refresh, SearchChats, TeamsFast},
    model::{self, Delivery, Message, Preview},
    state::Mode,
};
use gpui_kit::{
    base::TextView,
    component::{
        ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable,
        attachment::{
            Attachment, AttachmentContent, AttachmentDescription, AttachmentMedia, AttachmentTitle,
        },
        avatar::Avatar,
        badge::Badge,
        bubble::{Bubble, BubbleContent, BubbleReactions, BubbleVariant},
        button::{Button, ButtonVariants},
        empty::{Empty, EmptyContent},
        h_flex,
        input::{Input, Textarea},
        list::ListItem,
        menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
        message::{
            Message as MessageRow, MessageAlignment, MessageAvatar, MessageContent, MessageFooter,
            MessageGroup,
        },
        message_scroller::MessageScroller,
        resizable::{h_resizable, resizable_panel},
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{collections::HashMap, rc::Rc, sync::Arc};
use time::UtcOffset;

impl Render for TeamsFast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Signed out (or restoring a session): only the sign-in screen, no chat layout.
        let signed_out = self.state.mode != Mode::Live && self.state.chats.is_empty();
        let body = if signed_out {
            self.welcome(cx)
        } else {
            let sidebar = self.sidebar(window, cx);
            let conversation = self.conversation(window, cx);
            h_resizable("chat-layout")
                .child(
                    resizable_panel()
                        .size(px(290.))
                        .size_range(px(240.)..px(400.))
                        .child(sidebar),
                )
                .child(resizable_panel().child(conversation))
                .into_any_element()
        };
        v_flex()
            .id("teamsfast")
            .key_context("TeamsFast")
            .size_full()
            .text_color(cx.theme().foreground)
            .text_size(px(14.))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SearchChats, window, cx| {
                this.search.focus_handle(cx).focus(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NewConversation, window, cx| this.open_new_chat(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// Height of the unified title row; the traffic lights are centered in it (see main.rs).
const TITLE_HEIGHT: Pixels = px(56.);

impl TeamsFast {
    /// A unified title-bar region: dragging moves the window and a double click follows the
    /// system setting (zoom or minimize). Buttons inside stop the press from reaching it.
    fn title_area(&self, id: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .flex_shrink_0()
            .h(TITLE_HEIGHT)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        this.title_drag = true;
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.title_drag = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if std::mem::take(&mut this.title_drag) {
                    window.start_window_move();
                }
            }))
    }

    fn welcome(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let configured = crate::teams::validate_registration(
            &self.state.prefs.client_id,
            &self.state.prefs.tenant,
        )
        .is_ok();
        let signing = self.state.mode == Mode::SigningIn;
        let muted = cx.theme().muted_foreground;
        let action = if let (true, Some((code, url))) = (signing, self.state.device_code.clone()) {
            v_flex()
                .items_center()
                .gap_3()
                .child("Enter this code on Microsoft's sign-in page:")
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(code.clone()),
                        )
                        .child(
                            Button::new("copy-code")
                                .ghost()
                                .icon(IconName::Copy)
                                .tooltip("Copy code")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))
                                }),
                        ),
                )
                .child(
                    Button::new("open-device-page")
                        .primary()
                        .label("Open Microsoft sign-in")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .into_any_element()
        } else if signing {
            let page = self.state.sign_in_page.clone();
            v_flex()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Spinner::new())
                        .child(if page.is_some() {
                            "Finish signing in in your browser…"
                        } else {
                            "Signing in…"
                        }),
                )
                .when_some(page, |column, url| {
                    column.child(
                        Button::new("reopen-sign-in")
                            .ghost()
                            .small()
                            .label("Open the sign-in page again")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                })
                .into_any_element()
        } else {
            v_flex()
                .items_center()
                .gap_2()
                .when(!configured, |column| {
                    column
                        .child(Input::new(&self.client_id).w(px(320.)))
                        .child(Input::new(&self.tenant).w(px(320.)))
                })
                .child(
                    Button::new("sign-in")
                        .primary()
                        .large()
                        .label("Sign in with Microsoft")
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.start_sign_in(false, window, cx)
                            }),
                        ),
                )
                .child(
                    Button::new("sign-in-code")
                        .ghost()
                        .small()
                        .label("Sign in with a code instead")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_sign_in(true, window, cx)),
                        ),
                )
                .into_any_element()
        };
        let title = self.title_area("welcome-title", cx).w_full();
        let page =
            v_flex()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .gap_5()
                .child(
                    v_flex()
                        .items_center()
                        .gap_1()
                        .child(img(logo()).size(px(112.)))
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("TeamsFast"),
                        )
                        .child(div().text_color(muted).child(
                            "Microsoft Teams chats, fast. Use your work or school account.",
                        )),
                )
                .when_some(self.state.error.clone(), |column, error| {
                    column.child(
                        div()
                            .max_w(px(420.))
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                })
                .child(action)
                .when(signing, |column| {
                    column.child(
                        Button::new("cancel-sign-in")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.state.use_demo(false);
                                this.synchronize(window, cx);
                                cx.notify();
                            })),
                    )
                });
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(title)
            .child(page)
            .into_any_element()
    }

    fn sidebar(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let query = self.state.search.to_lowercase();
        let rows: Vec<_> = self
            .state
            .chats
            .iter()
            .filter(|chat| chat.summary.title.to_lowercase().contains(&query))
            .map(|chat| {
                let id = chat.summary.id.clone();
                let selected = self.state.selected.as_ref() == Some(&id);
                let muted = self.state.is_muted(&id);
                let title = chat.summary.title.clone();
                let preview = if !chat.draft.is_empty() {
                    format!("Draft: {}", chat.draft.replace('\n', " "))
                } else if chat.summary.preview_mine {
                    format!("You: {}", chat.summary.preview)
                } else {
                    chat.summary.preview.clone()
                };
                let label = format!(
                    "{title}, {} unread{}",
                    chat.unread,
                    if muted { ", muted" } else { "" }
                );
                ListItem::new(SharedString::from(id.clone()))
                    .selected(selected)
                    .accessibility_label(label)
                    .px_2()
                    .py_2()
                    .rounded_lg()
                    .h(px(58.))
                    .text_size(px(13.))
                    .child(
                        h_flex()
                            .gap_3()
                            .w_full()
                            .child(Badge::new().count(chat.unread).child(avatar(
                                &title,
                                self.state.photo(chat.summary.avatar_user.as_deref()),
                                px(30.),
                            )))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_weight(if chat.unread > 0 {
                                                        FontWeight::SEMIBOLD
                                                    } else {
                                                        FontWeight::MEDIUM
                                                    })
                                                    .child(title),
                                            )
                                            .when(muted, |row| {
                                                row.child(
                                                    Icon::new(IconName::Bell)
                                                        .size(px(12.))
                                                        .text_color(cx.theme().muted_foreground),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(model::time_label(
                                                        &chat.summary.updated_at,
                                                        self.state.offset,
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(preview),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_chat(id.clone(), window, cx)
                    }))
            })
            .collect();
        let (status, connected) = self.status();
        let status = status.to_owned();
        let header = self.title_area("sidebar-title", cx);
        v_flex()
            .size_full()
            // Translucent over the window's blurred background (vibrancy).
            .bg(cx.theme().sidebar.opacity(0.72))
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .px_3()
                    .pb_3()
                    .gap_1()
                    .child(
                        header
                            .flex()
                            .items_center()
                            .justify_end()
                            // Room for the traffic lights.
                            .pl(px(80.))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new("refresh-chats")
                                            .ghost()
                                            .small()
                                            .icon(IconName::RefreshCw)
                                            .tooltip("Refresh")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.refresh(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("new-chat")
                                            .ghost()
                                            .small()
                                            .icon(IconName::Plus)
                                            .tooltip("New conversation · ⌘N")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_new_chat(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("preferences")
                                            .ghost()
                                            .small()
                                            .icon(IconName::Settings)
                                            .tooltip("Settings · ⌘,")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_settings(window, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(Input::new(&self.search).prefix(IconName::Search).small()),
            )
            .child(
                v_flex()
                    .id("conversation-list")
                    .flex_1()
                    .min_h_0()
                    .px_2()
                    .gap_1()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(self.state.chats_loading, |list| {
                        list.child(
                            h_flex()
                                .p_4()
                                .gap_2()
                                .child(Spinner::new().small())
                                .child("Loading chats…"),
                        )
                    })
                    .when(
                        self.state.chats.is_empty() && !self.state.chats_loading,
                        |list| {
                            list.child(
                                div()
                                    .p_4()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No conversations yet."),
                            )
                        },
                    ),
            )
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().size(px(6.)).rounded_full().bg(if connected {
                                cx.theme().success
                            } else {
                                cx.theme().muted_foreground
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(status),
                            )
                            .when(self.state.mode == Mode::Demo, |row| {
                                row.child(Button::new("connect").small().label("Connect").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.open_connection(window, cx)
                                    }),
                                ))
                            }),
                    )
                    .when(!self.state.name.is_empty(), |footer| {
                        footer.child(div().text_xs().truncate().child(self.state.name.clone()))
                    }),
            )
    }

    fn conversation(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self
            .state
            .selected
            .as_ref()
            .and_then(|id| self.state.chats.iter().find(|chat| &chat.summary.id == id))
        else {
            return v_flex()
                .size_full()
                .bg(cx.theme().background)
                .child(self.title_area("empty-title", cx))
                .child(self.error_banner(cx))
                .child(
                    Empty::new()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Your conversations"),
                        )
                        .content(
                            EmptyContent::new()
                                .child("Connect your work account to start chatting."),
                        )
                        .child(
                            Button::new("connect-empty")
                                .primary()
                                .label(if self.state.mode == Mode::SigningIn {
                                    "Continue sign-in"
                                } else {
                                    "Connect account"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_connection(window, cx)
                                })),
                        ),
                )
                .into_any_element();
        };
        let id = chat.summary.id.clone();
        let title = chat.summary.title.clone();
        let header_photo = self.state.photo(chat.summary.avatar_user.as_deref());
        let subtitle = if self.state.mode == Mode::Demo {
            "Local demo".into()
        } else if chat.summary.members > 2 {
            format!("{} participants", chat.summary.members)
        } else {
            String::new()
        };
        let muted = self.state.is_muted(&id);
        let loading = chat.loading;
        let older = chat.next_messages.is_some();
        let send_error = chat.send_error.clone();
        let can_send = !chat.draft.trim().is_empty() && self.state.mode != Mode::SigningIn;
        let web_url =
            chat.summary.web_url.clone().filter(|value| {
                reqwest::Url::parse(value).is_ok_and(|url| url.scheme() == "https")
            });
        let transcript = self.transcripts.get(&id);
        let content = if let Some(transcript) = transcript {
            let rows = Rows {
                messages: transcript.messages.clone(),
                photos: self.state.photos.clone(),
                media: self.state.media.clone(),
                previews: self.state.previews.clone(),
                offset: self.state.offset,
                chat_id: id.clone(),
                chat_url: web_url.clone(),
                view: cx.entity().downgrade(),
            };
            MessageScroller::new(
                SharedString::from(format!("transcript-{id}")),
                transcript.scroll.clone(),
                move |index, _, cx| message(&rows, index, cx),
            )
            .flex_1()
            .min_h_0()
            .with_row_style(StyleRefinement::default().px_5().py(px(2.)))
            .with_content_style(StyleRefinement::default().py_3())
            .with_bottom_fade(cx.theme().background)
            .into_any_element()
        } else {
            Empty::new()
                .child(if loading {
                    "Loading messages…"
                } else {
                    "Start the conversation below."
                })
                .into_any_element()
        };
        let header = self.title_area("conversation-title", cx);
        v_flex()
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .child(
                header
                    .flex()
                    .items_center()
                    .px_5()
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(avatar(&title, header_photo, px(30.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        Button::new("mute-chat")
                            .ghost()
                            .small()
                            .icon(IconName::Bell)
                            .label(if muted { "Muted" } else { "" })
                            .tooltip(if muted {
                                "Unmute conversation"
                            } else {
                                "Mute conversation"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.toggle_mute(id.clone());
                                this.changed(cx);
                            })),
                    )
                    .when_some(web_url, |row, url| {
                        row.child(
                            Button::new("open-teams")
                                .ghost()
                                .small()
                                .icon(IconName::ExternalLink)
                                .tooltip("Open in Teams")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    })
                    .child(
                        Button::new("refresh-messages")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh messages")
                            .disabled(loading)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    ),
            )
            .child(self.error_banner(cx))
            .when(older, |column| {
                column.child(
                    h_flex().justify_center().py_1().child(
                        Button::new("older-messages")
                            .ghost()
                            .small()
                            .label(if loading {
                                "Loading history…"
                            } else {
                                "Load older messages"
                            })
                            .disabled(loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(id) = this.state.selected.clone() {
                                    this.state.load_older(id);
                                    this.synchronize(window, cx);
                                    cx.notify();
                                }
                            })),
                    ),
                )
            })
            .child(content)
            .when_some(send_error, |column, error| {
                column.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .mx_4()
                    .mb_4()
                    .mt_2()
                    .p_2()
                    .gap_2()
                    .items_end()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().input)
                    .child(
                        Textarea::new(&self.composer)
                            .appearance(false)
                            .bordered(false)
                            .aria_label("Message. Enter to send, Shift+Enter for a new line.")
                            .flex_1()
                            .disabled(self.state.mode == Mode::SigningIn),
                    )
                    .child(
                        Button::new("send-message")
                            .primary()
                            .small()
                            .icon(IconName::ArrowUp)
                            .tooltip("Send · Enter (Shift+Enter for a new line)")
                            .disabled(!can_send)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn error_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.state.error {
            Some(error) => h_flex()
                .px_4()
                .py_2()
                .gap_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                )
                .child(
                    Button::new("dismiss-error")
                        .ghost()
                        .small()
                        .label("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.error = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}

/// Everything a transcript row needs, captured once per render.
struct Rows {
    messages: Rc<Vec<Message>>,
    photos: HashMap<String, Option<Arc<Image>>>,
    media: Arc<HashMap<String, Option<Arc<RenderImage>>>>,
    previews: Arc<HashMap<String, Option<Preview>>>,
    offset: UtcOffset,
    chat_id: String,
    chat_url: Option<String>,
    view: WeakEntity<TeamsFast>,
}

fn open(url: Option<String>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, cx| {
        if let Some(url) = &url {
            cx.open_url(url);
        }
    }
}

fn react(
    view: &WeakEntity<TeamsFast>,
    chat_id: &str,
    message_id: &str,
    emoji: &str,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let (view, chat_id, message_id, emoji) = (
        view.clone(),
        chat_id.to_owned(),
        message_id.to_owned(),
        emoji.to_owned(),
    );
    move |_, window, cx| {
        let _ = view.update(cx, |this, cx| {
            this.state.toggle_reaction(&chat_id, &message_id, &emoji);
            this.synchronize(window, cx);
            cx.notify();
        });
    }
}

fn message(rows: &Rows, index: usize, cx: &mut App) -> AnyElement {
    let (messages, offset) = (&rows.messages, rows.offset);
    let value = &messages[index];
    let photo = rows.photos.get(&value.author_id).cloned().flatten();
    let previous = index.checked_sub(1).and_then(|i| messages.get(i));
    let next = messages.get(index + 1);
    let grouped = previous.is_some_and(|previous| value.grouped_after(previous, offset));
    let last_in_group = next.is_none_or(|next| !next.grouped_after(value, offset));
    let date = previous.is_none_or(|previous| {
        model::day_label(&previous.created_at, offset)
            != model::day_label(&value.created_at, offset)
    });
    let alignment = if value.mine {
        MessageAlignment::End
    } else {
        MessageAlignment::Start
    };
    let body_id = SharedString::from(format!("body-{}", value.id));
    let body = if !value.html.is_empty() {
        let media = rows.media.clone();
        (!value.text.trim().is_empty() || !value.images.is_empty()).then(|| {
            TextView::html(body_id, value.html.clone())
                .image_source(move |uri| match media.get(uri.as_ref()) {
                    Some(Some(image)) => ImageSource::Render(image.clone()),
                    // Not loaded yet: show nothing rather than let GPUI fetch and keep the
                    // full-size original; the worker's display-size copy replaces it.
                    _ => ImageSource::Custom(Arc::new(|_, _| None)),
                })
                .text_size(px(14.))
                .into_any_element()
        })
    } else {
        let markdown = if value.markdown.is_empty() {
            model::message_markdown(&value.text)
        } else {
            value.markdown.clone()
        };
        (!markdown.trim().is_empty()).then(|| {
            TextView::markdown(body_id, markdown)
                .text_size(px(14.))
                .into_any_element()
        })
    };
    let quote = value.quote.as_ref().map(|quote| {
        v_flex()
            .pl_2()
            .border_l_2()
            .border_color(cx.theme().muted_foreground)
            .text_size(px(12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(quote.author.clone()),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .line_clamp(2)
                    .child(quote.text.clone()),
            )
    });
    let files = value.files.iter().enumerate().map(|(ix, file)| {
        let kind = std::path::Path::new(&file.name)
            .extension()
            .map(|ext| ext.to_string_lossy().to_uppercase());
        let (url, description) = match (&file.url, &rows.chat_url) {
            (Some(url), _) => (
                Some(url.clone()),
                kind.map_or("Open file".into(), |kind| format!("{kind} · Open file")),
            ),
            (None, Some(chat)) => (Some(chat.clone()), "Open in Teams".into()),
            (None, None) => (None, "Attachment".into()),
        };
        Attachment::new()
            .id(SharedString::from(format!("file-{}-{ix}", value.id)))
            .small()
            .content(
                AttachmentContent::new()
                    .title(AttachmentTitle::new(file.name.clone()))
                    .description(AttachmentDescription::new(description)),
            )
            .on_click(open(url))
    });
    let preview = value
        .link
        .as_ref()
        .and_then(|link| rows.previews.get(link).cloned().flatten())
        .map(|preview| {
            let image = preview
                .image
                .as_ref()
                .and_then(|url| rows.media.get(url).cloned().flatten());
            Attachment::new()
                .id(SharedString::from(format!("preview-{}", value.id)))
                .w(px(380.))
                .max_w_full()
                .when_some(image, |card, image| {
                    card.media(AttachmentMedia::new().src(image))
                })
                .content(
                    AttachmentContent::new()
                        .title(AttachmentTitle::new(preview.title))
                        .description(AttachmentDescription::new(
                            if preview.description.is_empty() {
                                preview.site
                            } else {
                                format!("{} · {}", preview.site, preview.description)
                            },
                        )),
                )
                .on_click(open(Some(preview.url)))
        });
    let reactions = (!value.reactions.is_empty()).then(|| {
        BubbleReactions::new()
            .alignment(alignment)
            .children(value.reactions.iter().map(|reaction| {
                Button::new(SharedString::from(format!(
                    "reaction-{}-{}",
                    value.id, reaction.kind
                )))
                .ghost()
                .xsmall()
                .selected(reaction.mine)
                .label(if reaction.count > 1 {
                    format!("{} {}", reaction.emoji, reaction.count)
                } else {
                    reaction.emoji.clone()
                })
                .tooltip(if reaction.mine {
                    "Remove your reaction"
                } else {
                    "React"
                })
                .on_click(react(
                    &rows.view,
                    &rows.chat_id,
                    &value.id,
                    &reaction.emoji,
                ))
            }))
    });
    let wide = value.text.chars().count() > 160 || crate::html::has_blocks(&value.html);
    let bubble = Bubble::new()
        .alignment(alignment)
        .with_variant(if value.mine {
            BubbleVariant::Tinted
        } else {
            BubbleVariant::Secondary
        })
        .max_w(px(760.))
        // Block HTML (lists, tables) has no natural width, so a shrink-to-fit bubble would
        // collapse to its first short line. Long or structured messages take the full width.
        .when(wide, |bubble| bubble.w_full())
        .when(value.delivery == Delivery::Sending, |bubble| {
            bubble.opacity(0.7)
        })
        // The reaction pill overlaps the bubble's lower edge; keep room for it.
        .when(reactions.is_some(), |bubble| bubble.mb(px(14.)))
        .content(
            BubbleContent::new()
                .when(wide, |content| content.w_full())
                .bg(if value.mine {
                    cx.theme().accent
                } else {
                    cx.theme().secondary
                })
                .child(
                    v_flex()
                        .gap_2()
                        // Teams-style: sender and time head the bubble, so the avatar aligns with it.
                        .when(!value.mine && !grouped, |body| {
                            body.child(
                                h_flex()
                                    .gap_2()
                                    .text_size(px(12.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(value.author.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(value.time_label(offset)),
                                    ),
                            )
                        })
                        .children(quote)
                        .children(body)
                        .children(files)
                        .children(preview),
                ),
        )
        .when_some(reactions, |bubble, reactions| bubble.reactions(reactions));
    let menu = {
        let (view, chat_id, message_id, text) = (
            rows.view.clone(),
            rows.chat_id.clone(),
            value.id.clone(),
            value.text.clone(),
        );
        move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let menu = model::QUICK_REACTIONS
                .iter()
                .fold(menu, |menu, (emoji, label)| {
                    menu.item(
                        PopupMenuItem::new(format!("{emoji}  {label}")).on_click(react(
                            &view,
                            &chat_id,
                            &message_id,
                            emoji,
                        )),
                    )
                });
            let text = text.clone();
            menu.separator()
                .item(PopupMenuItem::new("Copy text").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }))
        }
    };
    let row = MessageRow::new()
        .id(SharedString::from(format!("message-{}", value.id)))
        .role(Role::ListItem)
        .alignment(alignment)
        .with_stack_style(StyleRefinement::default().gap(px(3.)))
        .when(!value.mine, |row| {
            row.avatar_slot(
                MessageAvatar::new()
                    .bg(rgba(0))
                    .self_start()
                    .child(if grouped {
                        div().size(px(28.)).into_any_element()
                    } else {
                        avatar(&value.author, photo, px(28.)).into_any_element()
                    }),
            )
        })
        .content(MessageContent::new().bubble(bubble))
        // Others' first message already shows its time in the bubble header.
        .when(
            value.delivery != Delivery::Sent || (last_in_group && (value.mine || grouped)),
            |row| {
                row.footer(
                    MessageFooter::new()
                        .content_inset(false)
                        .text_size(px(10.))
                        .text_color(if value.delivery == Delivery::Unconfirmed {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(match value.delivery {
                            Delivery::Sent => value.time_label(offset),
                            Delivery::Sending => "Sending…".into(),
                            Delivery::Unconfirmed => "Not confirmed".into(),
                        }),
                )
            },
        );
    v_flex()
        .gap(px(3.))
        .when(date, |column| {
            column.child(
                h_flex().justify_center().py_3().child(
                    div()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(cx.theme().muted)
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child(model::day_label(&value.created_at, offset)),
                ),
            )
        })
        .child(
            MessageGroup::new()
                .when(!grouped && index > 0, |group| group.pt_2())
                .child(
                    div()
                        .id(SharedString::from(format!("menu-{}", value.id)))
                        .context_menu(menu)
                        .child(row),
                ),
        )
        .into_any_element()
}

/// Kit 0.7.1 sizes the initials *box* (not its text) for custom pixel sizes, which leaves the
/// letters low and left. Set the text metrics ourselves so they sit in the middle.
/// The app icon, shown on the sign-in screen.
fn logo() -> Arc<Image> {
    static LOGO: std::sync::OnceLock<Arc<Image>> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/icon/logo-256.png").to_vec(),
        ))
    })
    .clone()
}

fn avatar(name: &str, photo: Option<Arc<Image>>, size: Pixels) -> Avatar {
    // Initials come from words, not symbols: "Product & design" is "PD", not "P&".
    let name: Vec<_> = name
        .split_whitespace()
        .filter(|word| word.chars().next().is_some_and(char::is_alphanumeric))
        .collect();
    Avatar::new()
        .name(name.join(" "))
        .with_size(size)
        .text_size(size * 0.42)
        .line_height(size * 0.5)
        .text_center()
        .whitespace_nowrap()
        .when_some(photo, |avatar, photo| avatar.src(photo))
}

pub(crate) fn apply_theme(light: bool, window: Option<&mut Window>, cx: &mut App) {
    use gpui_kit::component::{Theme, ThemeMode};
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        window,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_size = px(14.);
        theme.focus_ring = false;
        let colors = &mut theme.colors;
        let (background, sidebar, surface, foreground, muted, border, primary) = if light {
            (
                0xf5f6f8, 0xeceff3, 0xffffff, 0x242b36, 0x637084, 0xdbe0e8, 0x365fc7,
            )
        } else {
            (
                0x17191e, 0x202329, 0x2a2e36, 0xe9edf4, 0xa4adbb, 0x343a45, 0x8aaaf8,
            )
        };
        colors.background = rgb(background).into();
        colors.sidebar = rgb(sidebar).into();
        colors.sidebar_foreground = rgb(foreground).into();
        colors.foreground = rgb(foreground).into();
        colors.secondary = rgb(surface).into();
        colors.secondary_foreground = colors.foreground;
        colors.muted = rgb(if light { 0xe7ebf1 } else { 0x23272e }).into();
        colors.muted_foreground = rgb(muted).into();
        colors.border = rgb(border).into();
        colors.input = rgb(surface).into();
        colors.accent = rgb(if light { 0xdce5f7 } else { 0x303e58 }).into();
        colors.accent_foreground = colors.foreground;
        colors.list_active = colors.accent;
        colors.list_hover = colors.muted;
        colors.primary = rgb(primary).into();
        colors.primary_foreground = rgb(if light { 0xffffff } else { 0x172442 }).into();
        colors.button_primary = rgb(if light { 0x365fc7 } else { 0x496fd0 }).into();
        colors.button_primary_foreground = rgb(0xffffff).into();
        colors.ring = colors.primary;
        colors.caret = colors.primary;
        colors.title_bar = colors.sidebar;
        colors.title_bar_border = colors.border;
        colors.popover = colors.sidebar;
        colors.popover_foreground = colors.foreground;
        colors.group_box = colors.sidebar;
        colors.group_box_foreground = colors.foreground;
    });
}
