# GPUI migration assessment

Researched 2026-10-07. Research only: no dependency changes, build spike, compilation, native UI validation, or live Microsoft requests.

## Recommendation

Use published **GPUI Kit 0.7.1 on its normal GPUI snapshot as the baseline** for a short migration spike. It supplies the components that address the current interface complaint. Compare **GPUI Fast 0.1.0 as a pinned candidate** on that same screen; adopt it only if measured improvements justify its experimental status. “Baseline” does not mean a stable 1.0 API promise. These are separate decisions: a component-library migration can improve consistency without making a rendering-fork dependency mandatory.

## Versions and exact setup

Longbridge maintains GPUI Kit, including the styled `gpui-component` and unstyled `gpui-base` layers; GPUI originates at Zed. Latest verified Kit release: **0.7.1, 2026-10-05, `87d10ae`**, whose manifests pin the `gpui-pre-*` family to **`=0.3.8`**. Kit documents that earlier loose snapshot requirements caused incompatible upgrades. Use release documentation, not examples for older `InputState` APIs. [Release](https://github.com/longbridge/gpui-kit/releases/tag/v0.7.1), [manifest](https://github.com/longbridge/gpui-kit/blob/v0.7.1/Cargo.toml).

Proposed baseline manifest, not applied:

```toml
[dependencies]
gpui-kit = "=0.7.1"

[dev-dependencies]
gpui-kit = { version = "=0.7.1", features = ["test-support"] }
```

Use `gpui_kit::*` and `gpui_kit::component`. Defaults provide components and icons. Initialize once, register assets, and open the content using Kit's root-window helper. Kit enables the macOS platform `font-kit` feature; direct GPUI applications must enable that platform feature themselves or text can disappear. [Startup guide](https://gpui-kit.com/docs/getting-started/), [feature manifest](https://github.com/longbridge/gpui-kit/blob/v0.7.1/crates/kit/Cargo.toml).

Fast's first published release is **0.1.0, 2026-10-06**, revision **`598306fcbf77bff0f764b5a178a447344e72e132`**, incorporating Zed through `a1b71072e5b43faef437b471e988fbb5f972c99c`. Its release explicitly remains experimental. Direct applications alias published `gpui-fast` and `gpui-fast-platform`; Kit applications instead use same-name compatibility shims so Kit and application types share one core. [Fast release](https://github.com/longbridge/gpui-fast/releases/tag/v0.1.0).

For the Kit candidate, add at workspace root:

```toml
[patch.crates-io]
gpui-pre = { git = "https://github.com/longbridge/gpui-fast", rev = "598306fcbf77bff0f764b5a178a447344e72e132" }
gpui-pre-platform = { git = "https://github.com/longbridge/gpui-fast", rev = "598306fcbf77bff0f764b5a178a447344e72e132" }
gpui-pre-macros = { git = "https://github.com/longbridge/gpui-fast", rev = "598306fcbf77bff0f764b5a178a447344e72e132" }
gpui-pre-sum-tree = { git = "https://github.com/longbridge/gpui-fast", rev = "598306fcbf77bff0f764b5a178a447344e72e132" }
```

Patch `gpui-pre-web` and `gpui-pre-reqwest-client` to that same revision if present in the dependency graph. Add `extern crate gpui_kit as gpui;` at each consuming crate root for Fast's generated macro paths. The v0.1.0 shims advertise 0.3.8, matching Kit 0.7.1; mismatched versions can silently leave a patch unused. Lock the graph and inspect it before testing. This setup is verified from upstream manifests, **not compiled here**. [Compatibility instructions](https://github.com/longbridge/gpui-fast/blob/v0.1.0/compat/README.md), [shim manifest](https://github.com/longbridge/gpui-fast/blob/v0.1.0/compat/gpui-pre/Cargo.toml).

## Existing components that fit

| Need | Reuse and remaining application responsibility |
| --- | --- |
| Composer | `Textarea`/`TextareaState`, auto-grow and bounded scrolling. The user's new requirement is **Enter sends; Shift+Enter inserts a newline**. Use `submit_on_enter(true)` and connect submission to the send action; verify the Shift binding explicitly in the prototype. Cmd/Ctrl+Enter can remain an additional send shortcut. Verify IME confirmation and key repeat never send accidentally. [Textarea](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/base/primitives/textarea.md) |
| Timeline | `MessageScroller` already handles variable-height virtualization, tail following, prepended-history anchoring and unread navigation. Application supplies IDs, data and errors. Saved position resolves a message ID to an index; there is no public persisted pixel-offset API. [Scroller](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/component/message-scroller.md) |
| Rows and threads | `Message`, `MessageGroup`, `Bubble`, `Avatar`, `Attachment`, `Marker`. These arrange content; they do not implement Teams thread retrieval, reactions, delivery/read state or sender grouping policy. Keep those decisions in the model. [Message](https://gpui-kit.com/component/message/) |
| Selection | `TextView` is selectable; base selection coordinates multiple participants. Copying content outside mounted virtual rows requires application integration. Preserve plain-text fidelity and existing link safety; adding Markdown/HTML interpretation is a separate behavior change. [TextView](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/component/text-view.md), [selection](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/base/text-selection.md) |
| Settings | Dedicated `Settings`, `SettingPage`, `SettingGroup`, `SettingItem`, `SettingField`; searchable titles/descriptions/keywords and ordinary input/switch/dropdown controls. Storage, validation, credential access and save failures remain ours. [Settings](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/component/settings.md) |
| Shell | `TitleBar::window_options()` supplies platform setup; macOS uses native traffic lights. Theme tokens cover colors, typography and radii. Use the library defaults before custom controls. [TitleBar](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/component/title-bar.md), [theme](https://github.com/longbridge/gpui-kit/blob/v0.7.1/website/component/theme.md) |

AccessKit semantics still require stable identities, labels, roles and actual keyboard behavior. Native assistive-technology operation must be checked separately; headless tree assertions cannot prove screen-reader output. Keep the bundled Noto Sans Georgian, register it through `TextSystem::add_fonts`, and verify exact Georgian/emoji glyphs, selection and wrapping on each target. System font fallback alone is not a coverage guarantee. [Accessibility](https://gpui-kit.com/docs/accessibility/), [fonts](https://gpui-kit.com/docs/fonts/).

Current documented targets require macOS 15+, Windows 10+ with MSVC/SDK, and Linux graphical Wayland/X11 with Vulkan and native dependencies; Ubuntu 24.04 is the documented verified package baseline. This does not establish TeamsFast parity on those platforms. [Installation](https://gpui-kit.com/docs/installation/).

## Performance and development-loop evidence

Fast reports Linux release-build CPU frame costs of 3.31→0.19 ms for an unchanged window, 5.95→0.79 ms for sidebar scrolling, and 5.06→1.35 ms for streaming quotes. The first comparator disables retention in the same build; the latter two compare against `gpui-pre` 0.3.7. These are first-party showcase measurements, **not egui comparisons, Teams workloads, input-to-photon latency, or Graph delivery latency**. [Benchmark summary](https://github.com/longbridge/gpui-fast/blob/v0.1.0/README.md).

Important ceilings: untracked mutable state needs notification; retention is bypassed during accessibility, dragging and window refresh. Scroll layers bypass focused inputs, deferred content and frequently changing regions; their caches consume resources. Upstream provides oracle comparisons and headless/native checks, but they do not establish our app's correctness or macOS improvement. [Retention](https://github.com/longbridge/gpui-fast/blob/v0.1.0/docs/retained-mode.md), [scroll layers](https://github.com/longbridge/gpui-fast/blob/v0.1.0/docs/scroll-layers.md).

No evidence here establishes faster compilation. Kit warns that dependency optimization improves debug runtime while potentially increasing compilation time; first builds can take minutes. Keep plain Rust Kit, skip Shell/WebView/speech/extra grammars, and measure before adding profile tuning. `test-support` provides UI helpers; the manifest also distinguishes real-Metal rendering tests. [Installation](https://gpui-kit.com/docs/installation/), [features/tests](https://github.com/longbridge/gpui-kit/blob/v0.7.1/crates/kit/Cargo.toml).

## Migration seams and decision gate

Local inspection: [model](../src/model.rs) has no egui dependency; [teams](../src/teams.rs), [realtime](../src/realtime.rs), and [notifications](../src/notifications.rs) accept egui contexts principally for worker wake-ups. Preserve backend behavior, event contracts and stale-session guards; replace the wake-up seam without moving blocking HTTP onto GPUI. Request scheduling is a separate improvement described in the [performance plan](performance-and-ui-plan.md). [Settings](../src/settings.rs) serializes preferences/drafts through `eframe::Storage`; migrate that data without losing account scope while retaining OS-keyring service/key names. Keep one desktop package and the relay's server feature isolation. [Architecture](architecture.md), [Cargo](../Cargo.toml).

OpenMango's currently inspected [manifest](/Users/cpo/Developer/fun/openmango/openmango-rs/Cargo.toml) uses Kit 0.6.2 and platform 0.3.5. It is local experience, not a current compatibility template; its previously referenced migration document is absent.

Proposed short spike: one real demo-data screen with sidebar, long transcript, composer and Settings, first Kit baseline, then the identical screen with pinned Fast. No backend rewrite. Accept only after:

1. Native behavior preserves drafts on failure, newer text during sends, newline/submit/IME, copy, history anchoring, focus, narrow layout, Georgian, and Settings scrolling.
2. Record three warm edit→check/build/test cycles, a separately cached cold build, startup/RSS/idle CPU, and release p50/p95/p99 scrolling/typing frame times on the same machine/data. Compare with current egui; do not reuse old timings in [development notes](development.md) as current measurements.
3. Keep Kit if polish passes and iteration remains acceptable; add Fast only for a repeatable workload improvement with no behavior regression. If it fails, remove the patches/alias and restore the baseline lockfile.
4. Before full replacement, pass `just check`, native UI review and separate bundled-notification acceptance. Relay protocol and live Graph/OS checks remain distinct.
