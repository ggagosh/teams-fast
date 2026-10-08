use crate::{app::TeamsFast, settings::Settings as Preferences, state::Mode};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, IconName, TitleBar,
        button::Button,
        h_flex,
        input::Input,
        setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

struct PreferencesView {
    owner: WeakEntity<TeamsFast>,
    _subscription: Subscription,
}

impl TeamsFast {
    pub(crate) fn open_settings(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let owner = cx.entity();
        // The new window renders synchronously and reads TeamsFast, so open it after this update.
        cx.defer(move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(780.), px(650.)),
                    cx,
                ))),
                window_min_size: Some(size(px(640.), px(480.))),
                ..TitleBar::window_options()
            };
            let view = owner.clone();
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("TeamsFast Settings");
                cx.new(|cx| PreferencesView {
                    owner: view.downgrade(),
                    _subscription: cx.observe(&view, |_, _, cx| cx.notify()),
                })
            });
            owner.update(cx, |this, cx| {
                match opened {
                    Ok((window, _)) => this.settings_window = Some(window),
                    Err(error) => {
                        this.state.error = Some(format!("Could not open Settings: {error}"))
                    }
                }
                cx.notify();
            });
        });
    }
}

fn toggle(
    owner: &WeakEntity<TeamsFast>,
    title: &'static str,
    description: &'static str,
    get: fn(&Preferences) -> bool,
    set: fn(&mut Preferences, bool),
) -> SettingItem {
    let read = owner.clone();
    let write = owner.clone();
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| {
                read.upgrade()
                    .is_some_and(|view| get(&view.read(cx).state.prefs))
            },
            move |value, cx| {
                let _ = write.update(cx, |this, cx| {
                    set(&mut this.state.prefs, value);
                    crate::ui::apply_theme(this.state.prefs.light_theme, None, cx);
                    this.changed(cx);
                });
            },
        ),
    )
    .description(description)
}

impl Render for PreferencesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div()
                .child("The account window is closed.")
                .into_any_element();
        };
        let app = owner.read(cx);
        let live = app.state.mode == Mode::Live;
        let name = if live {
            app.state.name.clone()
        } else {
            "No account connected".into()
        };
        let relay_url = app.relay_url.clone();
        let relay_status = app.state.relay_status.clone();
        let watch_error = app.state.watch_error.clone();
        let progress = format!(
            "Watching {} of {} resources",
            app.state.watch_active, app.state.watch_total
        );
        let account_action = self.owner.clone();
        let connect_action = self.owner.clone();
        let test_notice = self.owner.clone();
        let status = app.status().0.to_owned();
        let main_window = app.main_window;
        let account = SettingPage::new("Account")
            .icon(IconName::User)
            .default_open(true)
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Microsoft 365")
                    .item(SettingItem::new(
                        "Signed in",
                        SettingField::render(move |_, _, _| div().child(name.clone())),
                    ))
                    .item(SettingItem::new(
                        "Connection",
                        SettingField::render(move |_, _, _| div().text_sm().child(status.clone())),
                    ))
                    .item(SettingItem::render(move |_, _, _| {
                        let account_action = account_action.clone();
                        Button::new("account-action")
                            .label(if live {
                                "Disconnect account…"
                            } else {
                                "Connect account"
                            })
                            .on_click(move |_, window, cx| {
                                window.remove_window();
                                let _ = main_window.update(cx, |_, window, cx| {
                                    let _ = account_action.update(cx, |this, cx| {
                                        this.settings_window = None;
                                        if live {
                                            this.open_disconnect(window, cx);
                                        } else {
                                            this.open_connection(window, cx);
                                        }
                                    });
                                });
                            })
                    })),
            );
        let notifications=SettingPage::new("Notifications").icon(IconName::Bell).resettable(false)
            .group(SettingGroup::new().title("Desktop alerts")
                .item(toggle(&self.owner,"Desktop notifications","Notify for new messages while TeamsFast is running.",|p|p.notifications,|p,v|p.notifications=v))
                .item(toggle(&self.owner,"Message previews","Include the sender and message text in alerts.",|p|p.notification_previews,|p,v|p.notification_previews=v))
                .item(toggle(&self.owner,"Quiet mode","Keep receiving messages without desktop alerts.",|p|p.quiet,|p,v|p.quiet=v))
                .item(SettingItem::render(move|_,_,_|{
                    let test_notice=test_notice.clone();
                    Button::new("test-notification").label("Send test notification").on_click(move|_,_,cx|{
                        let _=test_notice.update(cx,|this,_|{
                            if let Some(notices)=&this.state.notices {
                                notices.show(this.state.epoch(),this.state.selected.clone().unwrap_or_default(),"TeamsFast".into(),"Notifications are ready. Click to return to your conversation.".into());
                            }
                        });
                    })
                })));
        let appearance = SettingPage::new("Appearance")
            .icon(IconName::Sun)
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Interface")
                    .item(toggle(
                        &self.owner,
                        "Light appearance",
                        "Use the light theme. Turn off for dark appearance.",
                        |p| p.light_theme,
                        |p, v| p.light_theme = v,
                    ))
                    .item(toggle(
                        &self.owner,
                        "Link previews",
                        "Load titles and images of linked pages. The linked site sees your request.",
                        |p| !p.hide_link_previews,
                        |p, v| p.hide_link_previews = !v,
                    ))
                    .item(SettingItem::new(
                        "Keyboard",
                        SettingField::render(|_, _, _| {
                            div()
                                .text_sm()
                                .child("Enter sends · Shift+Enter inserts a new line")
                        }),
                    ))
                    .item(SettingItem::new(
                        "Navigation",
                        SettingField::render(|_, _, _| {
                            div()
                                .text_sm()
                                .child("⌘F Search · ⌘N New chat · ⌘R Refresh · ⌘, Settings")
                        }),
                    )),
            );
        let advanced = SettingPage::new("Advanced")
            .icon(IconName::Settings)
            .resettable(false)
            .description("Connection details and diagnostics.")
            .group(
                SettingGroup::new()
                    .title("Notification relay")
                    .item(SettingItem::new(
                        "Relay address",
                        SettingField::render(move |_, _, _| Input::new(&relay_url).w(px(320.))),
                    ))
                    .item(SettingItem::render(move |_, _, _| {
                        let connect_action = connect_action.clone();
                        Button::new("connect-relay")
                            .label("Reconnect live updates")
                            .disabled(!live)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    connect_action.update(cx, |this, cx| this.connect_relay(cx));
                            })
                    }))
                    .item(SettingItem::new(
                        "Relay",
                        SettingField::render(move |_, _, _| {
                            div().text_sm().child(relay_status.clone())
                        }),
                    ))
                    .item(SettingItem::new(
                        "Subscriptions",
                        SettingField::render(move |_, _, _| {
                            div().text_sm().child(progress.clone())
                        }),
                    ))
                    .item(SettingItem::render(move |_, _, cx| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .when_some(watch_error.clone(), |row, error| row.child(error))
                    })),
            );
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(h_flex().w_full().child("Settings")))
            .child(
                Settings::new("teamsfast-settings")
                    .sidebar_width(px(185.))
                    .pages([account, notifications, appearance, advanced]),
            )
            .into_any_element()
    }
}
