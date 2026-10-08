#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate gpui_kit as gpui;

use gpui_kit::*;

fn main() -> anyhow::Result<()> {
    let demo = std::env::args().any(|arg| arg == "--demo")
        || std::env::var("TEAMSFAST_DEMO").as_deref() == Ok("1");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/NotoSansGeorgian.ttf"
                ))])
                .expect("bundled Georgian font");
            cx.on_action(|_: &teamsfast::app::Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", teamsfast::app::Quit, None),
                KeyBinding::new("cmd-,", teamsfast::app::OpenSettings, Some("TeamsFast")),
                KeyBinding::new(
                    "cmd-k",
                    teamsfast::app::SwitchConversation,
                    Some("TeamsFast"),
                ),
                KeyBinding::new(
                    "cmd-shift-p",
                    teamsfast::app::ShowCommands,
                    Some("TeamsFast"),
                ),
                KeyBinding::new("cmd-/", teamsfast::app::ShowShortcuts, Some("TeamsFast")),
                KeyBinding::new(
                    "cmd-shift-f",
                    teamsfast::app::SearchChats,
                    Some("TeamsFast"),
                ),
                KeyBinding::new("cmd-n", teamsfast::app::NewConversation, Some("TeamsFast")),
                KeyBinding::new("cmd-r", teamsfast::app::Refresh, Some("TeamsFast")),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: "TeamsFast".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("Settings…", teamsfast::app::OpenSettings),
                        MenuItem::separator(),
                        MenuItem::action("Quit TeamsFast", teamsfast::app::Quit),
                    ],
                },
                Menu {
                    name: "Go".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action(
                            "Jump to Conversation…",
                            teamsfast::app::SwitchConversation,
                        ),
                        MenuItem::action("Commands…", teamsfast::app::ShowCommands),
                        MenuItem::action("New Conversation…", teamsfast::app::NewConversation),
                        MenuItem::action("Refresh", teamsfast::app::Refresh),
                    ],
                },
                Menu {
                    name: "Help".into(),
                    disabled: false,
                    items: vec![MenuItem::action(
                        "Keyboard Shortcuts",
                        teamsfast::app::ShowShortcuts,
                    )],
                },
            ]);
            let compact = std::env::args().any(|arg| arg == "--compact");
            let bounds = Bounds::centered(
                None,
                size(
                    px(if compact { 780. } else { 1120. }),
                    px(if compact { 580. } else { 780. }),
                ),
                cx,
            );
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(760.), px(520.))),
                app_id: Some("dev.teamsfast.desktop".into()),
                // Unified macOS layout: no title strip. The traffic lights sit in the sidebar's
                // 56 px header row; the app draws its own drag areas (`title_area` in ui.rs).
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(20.), px(21.))),
                }),
                app_owns_titlebar_drag: true,
                // Translucent sidebar over the desktop, like Finder and Mail.
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title("TeamsFast");
                cx.new(|cx| teamsfast::app::TeamsFast::new(demo, window, cx))
            })
            .expect("open TeamsFast window");
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
