# GPUI component integration

Verified 2026-10-07 against GPUI Kit 0.7.1 downloaded crate source and GPUI Fast 0.1.4 at `edbd7641e9cc5afae84c542e04a13b3171489875`. The user approved the full migration; the earlier research document's prototype gate is superseded. These are integration fragments, not a compiled example. No tests, builds or account requests were performed for this note. Main implementation validation is Clippy and the runnable native app.

## Dependency compatibility

Fast 0.1.4 was released October 7 at 11:44 UTC. Its compatibility packages still advertise 0.3.8, matching Kit 0.7.1's exact GPUI snapshot requirement. Use Kit as the application dependency and the same Fast revision for every replacement:

```toml
[dependencies]
gpui-kit = "=0.7.1"

[patch.crates-io]
gpui-pre = { git = "https://github.com/longbridge/gpui-fast", rev = "edbd7641e9cc5afae84c542e04a13b3171489875" }
gpui-pre-platform = { git = "https://github.com/longbridge/gpui-fast", rev = "edbd7641e9cc5afae84c542e04a13b3171489875" }
gpui-pre-macros = { git = "https://github.com/longbridge/gpui-fast", rev = "edbd7641e9cc5afae84c542e04a13b3171489875" }
gpui-pre-sum-tree = { git = "https://github.com/longbridge/gpui-fast", rev = "edbd7641e9cc5afae84c542e04a13b3171489875" }
```

Add matching `gpui-pre-web` or `gpui-pre-reqwest-client` patches only if those packages occur in the graph. At each consuming crate root, use `extern crate gpui_kit as gpui;` so Fast's generated macros resolve. Do not add a second unpatched GPUI core. [Release](https://github.com/longbridge/gpui-fast/releases/tag/v0.1.4), [compatibility](https://github.com/longbridge/gpui-fast/blob/v0.1.4/compat/README.md), [shim](https://github.com/longbridge/gpui-fast/blob/v0.1.4/compat/gpui-pre/Cargo.toml).

## Root, window and asynchronous events

```rust
use gpui_kit::*;
use gpui_kit::component::TitleBar;

application().with_assets(assets::Assets).run(|cx| {
    init(cx);
    open_window(TitleBar::window_options(), cx, |window, cx| {
        cx.new(|cx| TeamsView::new(window, cx))
    }).expect("open TeamsFast window");
});
```

`open_window<V: Render>(WindowOptions, &mut App, FnOnce(&mut Window, &mut App) -> Entity<V>)` returns `Result<(AnyWindowHandle, Entity<V>)>`. It installs `base::Root`; component initialization registers the dialog/sheet/notification/tooltip plugin. Root already renders `TextSelectionLayer`. Do not wrap another Root or add another selection layer. [Kit source](https://github.com/longbridge/gpui-kit/blob/v0.7.1/crates/kit/src/lib.rs), [component root](https://docs.rs/crate/gpui-component/0.7.1/source/src/root.rs).

Keep the event task on the view as a `Task<()>`. Existing worker channels remain authoritative; a bounded async wake channel merely wakes this loop. `drain_events` below denotes the application's existing event application path.

```rust
let event_task = cx.spawn_in(window, async move |view, cx| {
    while wake_rx.recv().await.is_ok() {
        if view.update_in(cx, |view, window, cx| {
            view.drain_events(window, cx);
            cx.notify();
        }).is_err() {
            break;
        }
    }
});
```

`Context::spawn_in` takes `AsyncFnOnce(WeakEntity<T>, &mut AsyncWindowContext) -> R`; it polls on the main thread. `WeakEntity::update_in` takes `&mut C: AppContext` and `FnOnce(&mut T, &mut Window, &mut Context<T>) -> R`, returning `Result<R>`. Awaiting the wake channel is appropriate; blocking HTTP is not. Do not silently drop the task. [Fast context](https://github.com/longbridge/gpui-fast/blob/v0.1.4/crates/gpui/src/app/context.rs), [weak entity](https://github.com/longbridge/gpui-fast/blob/v0.1.4/crates/gpui/src/app/entity_map.rs).

## Composer and search input

```rust
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));
let composer = cx.new(|cx| {
    TextareaState::new(window, cx).auto_grow(2, 8).submit_on_enter(true)
});
let subscription = cx.subscribe_in(&composer, window,
    |view, input, event: &InputEvent, window, cx| {
        match event {
            InputEvent::Change => view.set_draft(input.read(cx).value().to_string()),
            InputEvent::PressEnter { shift: false, .. } => view.send(window, cx),
            _ => {}
        }
    });
// Retain `subscription` and the input entities on the view.
Textarea::new(&composer).aria_label("Message");
Input::new(&search);
```

The shown policy is Enter to send, Shift+Enter for newline. `PressEnter { secondary: bool, shift: bool }` is emitted even when a newline was inserted, so filtering Shift matters. With `submit_on_enter(false)`, ordinary Enter also inserts a newline; do not treat every PressEnter event as a send. Existing Cmd/Ctrl+Enter behavior needs an explicit composer action if retained. Use one submission route and verify composition/IME in the native app.

Exact state calls:

```rust
composer.update(cx, |input, cx| {
    input.set_value(saved_draft, window, cx);
    input.set_disabled(false, cx);
});
let focus = composer.read(cx).focus_handle(cx);
window.focus(&focus, cx);
```

`set_value(impl Into<InputContent>, window, cx)` clears undo history and selection, emits no Change, and notifies. Use it for chat switches or send acknowledgements only when the model says the draft changed; never reset it every render. Keep editing enabled during a pending send to preserve the existing newer-draft behavior. [Input engine](https://docs.rs/crate/gpui-base/0.7.1/source/src/input/base/state.rs), [subscription signature](https://github.com/longbridge/gpui-fast/blob/v0.1.4/crates/gpui/src/app/context.rs).

## Transcript, grouping and plain text

```rust
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};

let scroller = cx.new(|cx| MessageScrollerState::new(messages.len(), cx));
// Observe the scroller on its owner, retaining the Subscription.
let scroll_subscription = cx.observe(&scroller, |_, _, cx| cx.notify());
MessageScroller::new("timeline", scroller.clone(), move |index, window, cx| {
    render_message(index, window, cx)
}).with_row_style(StyleRefinement::default().pb_2());
```

The renderer is `FnMut(usize, &mut Window, &mut App) -> E: IntoElement + 'static`; there is no `render_item` builder. Capture a shared message snapshot or a separate data entity; do not clone the entire transcript for each row. Do not read/update scroller state inside the row renderer while its internal list is borrowed.

After changing the data, update count and measurements together:

```rust
scroller.update(cx, |s, cx| { s.append(added, cx); });
scroller.update(cx, |s, cx| { s.prepend(older_added, cx); });
scroller.update(cx, |s, cx| { s.remeasure_items(changed..changed + 1, cx); });
scroller.update(cx, |s, cx| { s.scroll_to_item(first_unread, cx); });
scroller.update(cx, |s, cx| s.scroll_to_end(cx));
```

`append`, `prepend`, `splice(Range<usize>, count, cx)`, `remeasure_items`, and `scroll_to_item` return bool. `reset(count,cx)` re-engages tail following; reserve it for replacing a conversation, not routine updates. `remeasure(cx)` handles width/font changes. Unread meaning, ID-to-index mapping, loading/error rows and preserved per-chat position remain application-owned. [Actual scroller](https://docs.rs/crate/gpui-component/0.7.1/source/src/message_scroller.rs).

```rust
use gpui_kit::base::SelectableText;
use gpui_kit::component::{
    avatar::Avatar,
    message::{Message, MessageContent, MessageGroup, MessageHeader},
};

MessageGroup::new().child(
    Message::new()
        .avatar(Avatar::new().name(author.clone()))
        .header(MessageHeader::new().child(author).child(time))
        .content(MessageContent::new().child(
            SelectableText::new(("body", message_id), body)
        ))
)
```

`SelectableText` preserves plain text and participates in Root selection. `with_handle(id, TextSelectionHandle, text)` and `.document_order(u64)` allow related runs to share selection. Virtualized offscreen copy still needs participant/model integration. `MessageGroup` does not infer sender/time grouping; preserve existing `grouped_after` policy. [SelectableText](https://docs.rs/crate/gpui-base/0.7.1/source/src/selectable_text.rs), [Message](https://docs.rs/crate/gpui-component/0.7.1/source/src/message.rs).

Use `component::link::Link::new(id).href(url).child(label)` for existing validated links. `TextView` has `markdown(id,text)`, `html(id,text)` and `new(&Entity<TextViewState>)`, but no plain constructor. Its `on_link_click` is `Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static`; without a handler it opens URLs. Avoid accidentally interpreting Graph's already-normalized message text as markup. [Link](https://docs.rs/crate/gpui-component/0.7.1/source/src/link.rs), [TextView](https://docs.rs/crate/gpui-base/0.7.1/source/src/text/text_view.rs).

## Settings and dialogs

```rust
use gpui_kit::component::setting::{Settings, SettingPage, SettingGroup, SettingItem, SettingField};

Settings::new("preferences").pages(vec![
    SettingPage::new("Notifications").default_open(true).group(
        SettingGroup::new().title("Desktop alerts").item(
            SettingItem::new("Enable notifications", SettingField::switch(
                read_notifications, write_notifications,
            )).description("Show alerts for unread messages")
        )
    )
]);
```

`SettingField::switch/checkbox` take getter `Fn(&App)->bool`, setter `Fn(bool,&mut App)`. `input` uses `SharedString`; `dropdown` additionally takes `Vec<(SharedString,SharedString)>`; `scrollable_dropdown` handles longer options. Capture the application's state/weak handle explicitly, and notify after writes. Keep tokens and relay keys in the existing OS keyring. For existing retained `InputState` fields or action buttons, `SettingField::render` accepts `Fn(&RenderOptions,&mut Window,&mut App)->impl IntoElement`; it avoids recreating input state. [Settings fields](https://docs.rs/crate/gpui-component/0.7.1/source/src/setting/fields/mod.rs).

```rust
use gpui_kit::component::{WindowExt as _, dialog::DialogButtonProps};
window.open_dialog(cx, move |dialog, _, _| {
    dialog.title("Connection settings")
        .child(connection_form.clone())
        .button_props(DialogButtonProps::default().ok_text("Done"))
});
// window.close_dialog(cx);
```

The builder is `Fn(Dialog,&mut Window,&mut App)->Dialog + 'static`, potentially rerun; create entities outside it and clone handles inside. `Dialog::footer` takes an element, not the older footer callback. `DialogButtonProps::on_ok`/`on_cancel` return bool: true closes, false keeps open. Root owns focus restoration. [WindowExt](https://docs.rs/crate/gpui-component/0.7.1/source/src/window_ext.rs), [Dialog](https://docs.rs/crate/gpui-component/0.7.1/source/src/dialog/dialog.rs).

## Sidebar, resizing and menus

```rust
use gpui_kit::component::{
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem},
    resizable::{h_resizable, resizable_panel},
    button::Button, menu::DropdownMenu as _,
};

let navigation = Sidebar::new("navigation").child(
    SidebarGroup::new("Chats").child(
        SidebarMenu::new().child(SidebarMenuItem::new("All chats").active(true))
    )
);
h_resizable("chat-split")
    .child(resizable_panel().size(px(280.)).size_range(px(220.)..px(420.)).child(navigation))
    .child(resizable_panel().child(conversation));
Button::new("actions").label("More").dropdown_menu(|menu, _, _| {
    menu.menu("Settings", Box::new(OpenSettings))
});
```

Actual source requires `Sidebar::new(id)` and `h_resizable(id)`: some website examples use stale signatures. `SidebarMenuItem::on_click` takes `Fn(&ClickEvent,&mut Window,&mut App)`. `DropdownMenu` is a trait; its builder receives `PopupMenu,&mut Window,&mut Context<PopupMenu>`. `OpenSettings` above is an application action. [Sidebar](https://docs.rs/crate/gpui-component/0.7.1/source/src/sidebar/mod.rs), [resizable](https://docs.rs/crate/gpui-component/0.7.1/source/src/resizable.rs), [menu](https://docs.rs/crate/gpui-component/0.7.1/source/src/menu/dropdown_menu.rs).

## Other existing controls worth using

| UI need | Pinned API |
| --- | --- |
| Searchable people/chat list | `list::{List,ListState,ListDelegate,ListItem}`; `ListState::new(delegate,window,cx)`, `List::new(&state)`. Delegate supplies `items_count(section,cx)`, `render_item(IndexPath,window,cx)->Option<Item>` and optional `perform_search(...)->Task<()>`. Rows are documented as uniform height; use MessageScroller for mixed-height messages. |
| Anchored details | `popover::Popover::new(id).trigger(Button::new(id)).content(|state,window,cx| element)`. Trigger must implement `Selectable`; content state is `PopoverState`. |
| Loading | `spinner::Spinner::new()`; button loading state for a specific action. |
| Empty or failure state | `empty::{Empty,EmptyHeader,EmptyTitle,EmptyDescription,EmptyContent}`. Compose message and action rather than hand-drawing a panel. |
| Unread/status indicator | `badge::Badge::new().count(unread)` or `.dot()`; zero counts hide. Pair dots with meaningful state text. |
| Toggle | `switch::Switch::new(id).checked(value).label(text).on_change(...)`; callback receives `&bool,window,cx`. `checkbox::Checkbox` is the parallel checkbox control. |
| Help | `Button::tooltip(text)`; custom element tooltips use `tooltip::Tooltip`. A tooltip supplements an accessible label. |
| Local feedback | `WindowExt::push_notification(Notification::info(text),cx)`. Preserve existing OS notification transport/click routing separately. |

These entries were checked in the downloaded `gpui-component-0.7.1/src` modules. [Versioned source index](https://docs.rs/crate/gpui-component/0.7.1/source/src/). Reuse these before adding custom controls. Keep semantic theme tokens, stable IDs and keyboard focus behavior; no unrelated component inventory is needed.
