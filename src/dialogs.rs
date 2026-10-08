use crate::{
    app::{DialogKind, TeamsFast},
    state::{Mode, NewChat},
};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, IconName, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        dialog::Dialog,
        h_flex,
        input::{Input, Textarea},
        list::ListItem,
        spinner::Spinner,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

impl TeamsFast {
    pub(crate) fn edit_message(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self
            .state
            .chats
            .iter()
            .find(|c| c.summary.id == chat_id)
            .and_then(|c| c.messages.iter().find(|m| m.id == message_id))
        else {
            return;
        };
        if !message.files.is_empty() || message.quote.is_some() || !message.images.is_empty() {
            self.state.error =
                Some("Edit messages with attachments, images or quotes in Teams.".into());
            cx.notify();
            return;
        }
        let target = (chat_id.clone(), message_id.clone());
        if self.editing.as_ref() != Some(&target) {
            let text = message.text.clone();
            self.message_edit
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.editing = Some(target);
        let input = self.message_edit.clone();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            let (chat_id, message_id) = (chat_id.clone(), message_id.clone());
            dialog
                .title("Edit message")
                .width(px(520.))
                .child("Saved as plain text; existing formatting will be removed.")
                .child(Textarea::new(&input).aria_label("Message text"))
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-edit")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("save-edit")
                                .primary()
                                .label("Save changes")
                                .disabled(input.read(cx).value().trim().is_empty())
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        let text = this.message_edit.read(cx).value().to_string();
                                        if !text.trim().is_empty() {
                                            this.state.mutate_message(
                                                &chat_id,
                                                &message_id,
                                                crate::model::MessageChange::Edit(text),
                                            );
                                            this.synchronize(window, cx);
                                            cx.notify();
                                        }
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
        self.message_edit.focus_handle(cx).focus(window, cx);
    }

    pub(crate) fn delete_message(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let (chat_id, message_id) = (chat_id.clone(), message_id.clone());
            dialog
                .title("Delete message?")
                .width(px(420.))
                .child("This deletes your message for everyone in this conversation.")
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-delete")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("delete-message")
                                .danger()
                                .label("Delete message")
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.state.mutate_message(
                                            &chat_id,
                                            &message_id,
                                            crate::model::MessageChange::Delete,
                                        );
                                        this.synchronize(window, cx);
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Account actions start the browser sign-in; progress shows on the sign-in screen.
    pub(crate) fn open_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_sign_in(false, window, cx);
    }

    pub(crate) fn open_new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state.mode, Mode::Demo | Mode::Live) {
            return;
        }
        if self.state.new_chat.is_none() {
            self.state.new_chat = Some(NewChat::default());
            self.people_query
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.topic
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.dialog = Some(DialogKind::NewChat);
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            new_chat(dialog, &view, window, cx)
        });
        cx.notify();
    }

    pub(crate) fn open_disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.disconnect_open = true;
        self.dialog = Some(DialogKind::Disconnect);
        let view = cx.entity().downgrade();
        window.open_dialog(cx,move|dialog,_,_| {
            let view=view.clone();
            dialog.title("Disconnect account?").width(px(420.)).child("Saved sign-in will be removed. Your drafts remain saved locally for this account.")
                .footer(h_flex().justify_end().gap_2()
                    .child(Button::new("cancel-disconnect").label("Keep working").on_click(|_,window,cx|window.close_dialog(cx)))
                    .child(Button::new("disconnect").danger().label("Disconnect").on_click(move|_,window,cx|{
                        let _=view.update(cx,|this,cx|{
                            this.state.use_demo(true);this.synchronize(window,cx);this.changed(cx);
                        });
                        window.close_dialog(cx);
                    })))
        });
    }
}

fn new_chat(dialog: Dialog, view: &WeakEntity<TeamsFast>, _: &mut Window, cx: &mut App) -> Dialog {
    let Some(entity) = view.upgrade() else {
        return dialog;
    };
    let app = entity.read(cx);
    let Some(state) = &app.state.new_chat else {
        return dialog;
    };
    let query = app.people_query.clone();
    let topic = app.topic.clone();
    let selected = state.selected.clone();
    let people = state.people.clone();
    let loading = state.loading;
    let creating = state.creating;
    let error = state.error.clone();
    let results = people
        .into_iter()
        .map(|person| {
            let view = view.clone();
            let checked = selected.iter().any(|selected| selected.id == person.id);
            ListItem::new(SharedString::from(person.id.clone()))
                .selected(checked)
                .accessibility_label(person.display_name.clone())
                .py_2()
                .child(v_flex().child(person.display_name.clone()).when_some(
                    person.mail.clone().or(person.user_principal_name.clone()),
                    |row, email| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(email),
                        )
                    },
                ))
                .when(checked, |row| {
                    row.child(gpui_kit::component::Icon::new(IconName::Check))
                })
                .on_click(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(dialog) = &mut this.state.new_chat {
                            if let Some(index) = dialog
                                .selected
                                .iter()
                                .position(|selected| selected.id == person.id)
                            {
                                dialog.selected.remove(index);
                            } else {
                                dialog.selected.push(person.clone());
                            }
                        }
                        cx.notify();
                    });
                })
        })
        .collect::<Vec<_>>();
    let names = selected
        .iter()
        .map(|person| person.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let create = view.clone();
    dialog
        .title("New conversation")
        .width(px(480.))
        .overlay_closable(!creating)
        .child(
            v_flex()
                .gap_3()
                .child(
                    Input::new(&query)
                        .prefix(IconName::Search)
                        .disabled(creating),
                )
                .when(!names.is_empty(), |form| {
                    form.child(div().text_sm().child(format!("To: {names}")))
                })
                .child(
                    v_flex()
                        .id("people-results")
                        .max_h(px(230.))
                        .overflow_y_scroll()
                        .children(results)
                        .when(loading, |list| {
                            list.child(
                                h_flex()
                                    .gap_2()
                                    .p_3()
                                    .child(Spinner::new().small())
                                    .child("Searching…"),
                            )
                        }),
                )
                .when(selected.len() > 1, |form| {
                    form.child(Input::new(&topic).disabled(creating))
                })
                .when_some(error, |form, error| {
                    form.child(div().text_sm().text_color(cx.theme().danger).child(error))
                }),
        )
        .footer(
            Button::new("create-chat")
                .primary()
                .label("Start conversation")
                .disabled(selected.is_empty() || creating)
                .loading(creating)
                .on_click(move |_, window, cx| {
                    let _ = create.update(cx, |this, cx| this.create_chat(window, cx));
                }),
        )
}
