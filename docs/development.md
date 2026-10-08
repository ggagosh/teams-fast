# Development and checks

Use Rust 1.95 or newer and `just`. Keep `Cargo.lock` and the pinned GPUI Fast revision.

```sh
just dev
just dev --demo
just dev --demo --compact
just check
```

Debug builds compile dependencies with `opt-level = 3`, because GPUI is unusably slow unoptimized. `cargo run` and `just app` sign the debug build with your Apple Development identity (`scripts/sign_dev.sh`, through the Cargo runner in `.cargo/config.toml`). Keychain access therefore survives rebuilds; set `TEAMSFAST_DEV_SIGNING_IDENTITY` to choose another identity. Debug builds keep their sign-in and rich-notification key in the Keychain under `dev.teamsfast.desktop.debug`, separate from the release app's `dev.teamsfast.desktop`. The two builds are signed by different identities, so shared items would make macOS ask for the password whenever the other build had written last. Sign in once in each.

`just dev` loads the ignored `.env`; copy `.env.example` for a new checkout. It holds only public IDs and the relay URL. Release builds bake the same values in at compile time from the CI variables, so the app opens straight to **Sign in with Microsoft**. Plain Cargo uses inherited environment variables. Demo mode ignores saved account settings and does not persist its changes. Compact mode opens at 780 × 580 logical pixels; the minimum window is 760 × 520.

`just check` runs formatting, workspace/all-target compilation, and Clippy with warnings denied. Per the user's migration instructions, do not add new tests. Existing backend tests remain, but they were not run in this migration; egui UI tests were removed with the old frontend. Native interaction is checked in the runnable app.

The desktop is the default workspace member, so normal desktop builds do not compile the relay server. GPUI Kit is pinned to 0.7.1; all patched Fast compatibility crates use the same revision from `Cargo.toml`.

## Native macOS app

```sh
just app          # or: just app --demo
```

This builds `target/TeamsFast.app`, signs it with the Apple Development identity, and runs it from the terminal. `.env` and the trace output work as with `just dev`. OS notifications and the Dock badge need this bundle; `just dev` cannot show them. The first launch asks for notification permission. If it was denied before, enable TeamsFast (including badges) in System Settings → Notifications. Omit `--demo` to restore a remembered account. Launching the bundle with `open` does not load `.env`; the current local bundle can include public IDs and the relay URL in its generated `LSEnvironment`, or configuration can be entered in the app. Never embed keys or tokens in the bundle. Signing/notarization remain release work.

Check the runnable app:

1. Resize the sidebar and window; check the Kit titlebar, transcript, and composer.
2. Switch chats with drafts, send a demo message using Enter, and add multiline text with Shift+Enter.
3. Check Georgian, emoji, long content, selection/copying, inline links, attachment cards, and keyboard focus.
4. Open Settings with Cmd+, and check Account, Notifications, Appearance, and Advanced, including the light/dark switch.
5. Check older-history prepending, scrolling, pending sends, and visible errors.
6. In the bundled app, use **Send test notification** and check OS permission, delivery, and click routing separately. The in-app switch does not grant OS permission. Settings now reports whether macOS denied permission, rejected posting, or accepted the request, and offers **Open macOS Notification Settings…**. Accepted does not mean displayed: check Focus and alert presentation too.
7. Close the main window with its red button. Confirm the app stays running without marking hidden-window activity read; reopen from the Dock and verify drafts and open edit text. ⌘Q must still use the save-before-quit gate.
8. Complete the signed-in checks in [the milestone](initial-milestone.md) using an approved conversation.

## Window initialization and startup acceptance

The content constructor passed to `gpui_kit::open_window` runs **before** Kit installs the window Root and component dialog state. The initial `TeamsFast::synchronize` is deferred with `cx.defer_in`; calling it inside the constructor crashed when a remembered account triggered dialog cleanup. Account-switch cleanup remains in the shared synchronization path after initialization.

Check three native startup paths: demo, a fresh profile, and a remembered account with `cached_account` matching the tenant/client prefix. Demo/fresh startup alone does not exercise restoration. A temporary isolated-home fixture with synthetic account metadata reproduced the released 0.4.1 abort and survived startup with the fix; no tenant history or credential reset was needed. The fixed, locally Developer-ID-signed updater bundle passed all three startup checks, `just check`, updater-enabled Clippy and all 11 existing tests. This is launch verification, not full Graph/notification or update-install acceptance. No permanent tests were added.

## Chat visibility, activity and close-to-hide acceptance

- **Local checks:** `just check`, updater-enabled Clippy and all 11 existing tests pass. Temporary isolated-source fixtures reproduce a false unread badge on the published source (system-event preview yields 1) and pass on the fix (0). They also cover ordinary/deleted-message previews, hidden/local-work visibility, event and legacy cache decoding, complete multi-page background refreshes, partial failures, stale replies, late cache arrival, unavailable-chat send protection, read-intent cancellation, and deferring a new chat's first message until visibility is confirmed without counting it twice. No permanent tests or tenant data were added.
- **Native UI:** an Apple-Development-signed bundle survived demo, fresh and synthetic remembered-account startup. PID-targeted native close-button actions hid the app without terminating it; Dock-equivalent reopen restored the same process/window, and native ⌘Q exited. Demo rendering and Settings opening were inspected. Background input did not establish a draft, so native draft/edit-text retention remains a manual acceptance item, not a claimed pass.
- **Release preflight (0.5.0):** default/updater-enabled checks and all 11 tests passed again after the version bump. The locally Developer-ID-signed, updater-enabled release bundle survived remembered-account, fresh and demo startup. This is not Intel runtime or full update-install/relaunch acceptance.
- **Call-only filtering preflight (0.5.1):** after the version bump, `just check`, updater-enabled Clippy, all 11 existing tests and temporary meeting-history fixtures passed. The fixtures cover full/partial/failed scans, real messages on older pages, cache evidence, stale results, new activity, the 30-day cutoff, search eligibility, drafts/pending sends, and avoiding unread changes from membership timestamps. The Apple-Development-signed bundle survived remembered-account, fresh and demo startup. Native search interaction and the reported live-account example still require acceptance; fixture success is not Teams sidebar parity. No permanent tests were added.
- **Evolvable-event correction (0.5.2):** a read-only live Graph comparison reproduced the failure on a reported call-only meeting: without `Prefer: include-unknown-enum-members`, its structured system events were returned as `unknownFutureValue` and the 0.5.1 inspector incorrectly reported real messages. With the corrected shared decoder/header, the same thread is excluded from the default-list eligibility check but remains searchable; sampled meetings with actual messages remain listed. The poisoned cache field is ignored, not the history/drafts. `just check`, updater-enabled Clippy, all 11 existing tests and temporary enum/cache/visibility fixtures pass. A signed development bundle survives remembered, fresh and demo startup. This verifies live read-only decoding and eligibility, not an installed-upgrade/native-search interaction. No chat/read/visibility mutations, database updates, credential writes, or relay registration were performed by the diagnostic; tokens and message bodies were not logged.
- **Relay protocol:** not rerun; relay code and protocol are unchanged.
- **Live Graph / OS:** the development bundle displayed the new explicit macOS permission-denied error; permission was not changed. Banner delivery/click routing, the installed release's permission state, live hidden-chat reconciliation and other-device read parity remain unverified. No real Teams messages were sent or modified. See [chat status](chat-status.md) for the supported API boundary.

## Encrypted history and optimistic updates

See [local history](local-history.md) for the storage/reconciliation contract and acceptance cases. `rusqlite` bundles SQLCipher; native macOS builds use system cryptography. The cache worker verifies `cipher_version` before storing content. Debug and release history files and Keychain keys are isolated.

Local checks for this change: `just check` and all 11 existing workspace tests passed. A temporary synthetic database using the linked SQLCipher 4.10.0 library verified encrypted database/WAL content, correct-key reopening, wrong-key rejection, integrity, FTS5 availability, and retention that keeps pending sends. This did not exercise Keychain or live account migration. No permanent tests were added.

Native demo rendering was inspected in a PID-targeted screenshot; attempted background input did not take effect, so edit/delete interaction acceptance remains unverified. No real Teams messages were sent or modified. Relay protocol checks were not rerun (no relay changes). Live Graph read sync, failure reconciliation and OS notifications still require acceptance in the signed bundle.

## Keyboard navigation acceptance

In `--demo`, check ⌘K with an empty query, an exact name, a non-contiguous name (for example `pd` → Product & design), and no matches. Use arrows and Return; confirm the correct chat opens and typing goes to its composer. Escape first clears the query and then closes, restoring prior focus. Mouse selection must behave the same. Visit several chats, reopen the switcher, and check recent ordering; the current chat is last.

Check ⌘⇧P from the composer and while the switcher is open. Select **New conversation**, **Settings**, **Keyboard shortcuts**, and appearance/mute commands; opening a replacement dialog must not immediately close it. Disabled commands cannot run. Check ⌘/, ⌘N, ⌘R, ⌘, and ⌘⇧F; ⌘F is deliberately unbound. Repeat at the minimum window size, with a draft, offline, and after an account change. All modal snapshots must disappear on account change.

The keyboard revision passed `just check`, all 11 existing workspace tests, and eight temporary assertions against the actual fuzzy matcher (exact/prefix/subsequence, Unicode, whitespace and no-match cases). No permanent tests were added. Native demo rendering and the sidebar shortcut hint were inspected; background keyboard/AX attempts did not provide a reliable palette/focus acceptance result. Full native keyboard interaction remains manual acceptance, not a claimed automated pass. Relay and live Graph/OS checks are separate and were not rerun for keyboard navigation.

### Palette styling follow-up (0.5.3)

The switcher and command palette share the dialog's themed surface and separated keyboard hints. Conversation rows use compact, single-line titles and the same avatar unread badges as the sidebar; command labels align without mixed icon insets. Ranking, filtering and action routing are unchanged.

Temporary signed demo fixtures checked dark/light rendering, long-name truncation, unread badges, no-match results and Commands. PID-targeted Down then Return opened the expected conversation and showed composer focus. Queries in the visual fixtures were supplied programmatically; typed search/Escape, minimum-window and real-account acceptance remain manual. `just check` and all 11 existing tests passed; the signed development bundle survived remembered-account, fresh and demo startup with isolated synthetic profiles. Relay, live Graph and OS notification checks were not rerun for this styling change.

## Automatic updates

See [automatic updates](updates.md) for the native Sparkle integration, release feeds, key custody, safe restart and acceptance requirements. Plain development builds intentionally omit the updater framework; release builds enable it. The additional CI secret is `SPARKLE_PRIVATE_KEY`, backed up in 1Password, never `.env`. The release script signs nested Sparkle helpers/framework before the outer app, notarizes/staples, then generates separate signed appcasts for Apple Silicon and Intel.

## Relay protocol check

```sh
python3 scripts/check_relay.py https://teamsfast-relay.omedialab.com
```

This stdlib-only script prompts for the key, creates a temporary registration, checks metadata-only webhook delivery and immediate long-poll wake-up, and removes the registration. It sends no Teams messages and does not prove Microsoft subscription delivery.

## Latency diagnostics

```sh
TEAMSFAST_TRACE_LATENCY=1 just app
```

This opt-in trace prints request kind (`send`, `incoming`, or `read`) with queue/work milliseconds, plus `relay:` connection status, `watch:` subscription progress/errors, and `push:` batches (change count and kinds). A healthy incoming message shows `push: 1 change(s) ["created"]` followed by `incoming:`. New messages then print `delivery: microsoft+relay=…ms fetch=…ms total=…ms`: message creation (Microsoft's timestamp) to push arrival, push arrival to the fetched message, and the sum. It compares the local clock with Microsoft's, so accuracy depends on clock sync (macOS keeps it within tens of milliseconds). The bundle also prints `notifications:` permission status and `watch: rich notifications enabled` (or `rejected (HTTP n)`). A rich push that can't be used prints `rich: fetching instead (reason)`. The reason is a fixed phrase or an HTTP/network error, never message content. With rich notifications working, `incoming:` work time and the `fetch=` part drop to a few milliseconds. The self chat ("Name (You)") is not covered by Graph change notifications, so it never produces `push:` lines. It excludes chat/message IDs, bodies, and credentials. Queue time includes waiting for the lane's start budget; work time includes the request and any required authentication work. It does not measure Microsoft's event-generation delay. Record comparable runs before claiming a speedup.

## Verification record

Rich notifications, live (owner's account, relay redeployed on Coolify): the subscription was accepted with the encryption certificate. After the first message, incoming messages were decrypted locally in 1–7 ms instead of fetched (previously 340–650 ms). Total delivery from Microsoft's timestamp to on screen was 300–640 ms, almost all of it Microsoft plus the relay. The first message after launch took about 415 ms because Microsoft's signing keys were fetched once. `updated` pushes also arrived as rich content. OS notification and Dock badge display were not part of this run.

The later request-lane and bubble-layout revision passed formatting, compilation and Clippy. No tests were added or run. Its native demo preview showed the new surfaces, aligned message bubbles, compact composer, and inline links. Source review closed the credential-save/sign-out race, stale-history ordering, and retained-row measurement findings after fixes. Actual send/notification latency still needs a controlled live measurement; no measured speedup is claimed.

The GPUI migration passed formatting, workspace/all-target compilation, and Clippy with warnings denied on 2026-10-07. The initial debug executable build took 53.41 seconds with an existing shared dependency cache; this was not a clean build. No new tests were written and no tests were run for the migration.

The native GPUI bundle launched, restored the saved signed-in account automatically, and rendered Georgian messages. Inspection found chat-row layout issues; those were corrected along with grouped-avatar spacing and duplicate link controls, and formatting/compilation/Clippy passed again. Full layout and interaction acceptance remains with the user's manual QA. No Microsoft messages were sent by the assistant. Enter/Shift+Enter behavior, message sending, and OS notification delivery/click routing remain for the user to verify.

Earlier egui UI screenshots and test counts describe the replaced frontend, not GPUI validation. The existing relay previously passed public HTTPS health, authentication, Graph validation echo, invalid-state rejection, and synthetic webhook delivery with a 0.12-second wake-up. That measurement excluded Microsoft's notification generation and the subsequent Graph message fetch. It was not rerun as part of the UI migration.

The earlier migration bundle was reopened and its compact chat rows, grouped-message layout, saved account, and Georgian rendering were observed. A bounded source review closed with no remaining blocker from its findings; native interaction QA remains with the user.

| Review finding | Final disposition |
| --- | --- |
| Reopening chat creation could discard an in-flight operation | Resolved in source: reopen preserves the operation and duplicate-request guard |
| Reading to the bottom did not clear unread counts | Resolved in source: active-window and scroll position are reconciled by the existing event/timer path |
| Search text could disagree with the filtered list after reset | Resolved in source: retained input follows model resets |

Account changes also clear the UI's cached transcripts before displaying another account.

The owner reported basic signed-in and live-update setup working. A complete recorded Graph-message/OS-notification acceptance run remains pending. Windows/Linux behavior and any claimed performance improvement remain unverified.

## Where changes belong

Use `src/ui.rs` for Kit composition, `src/app.rs` for the GPUI controller, `src/state.rs` for UI-independent state, `src/dialogs.rs` and `src/preferences.rs` for their respective surfaces, and `src/teams.rs` for Microsoft integration. Follow [DESIGN.md](../DESIGN.md). Use ast-grep for Rust structure and `rg` for text.

## CI and releases

GitHub Actions run `.github/workflows/ci.yml` on pushes and pull requests: formatting, Clippy, the relay tests, and a standalone relay build from `relay/Cargo.lock`. Pushing a tag `vX.Y.Z` that matches `Cargo.toml` runs `.github/workflows/release.yml`. It builds Apple Silicon and Intel apps with `scripts/release_macos.sh`, signs them with Developer ID (hardened runtime), notarizes and staples them, and publishes the zips to GitHub Releases.

The release secrets live only in GitHub Actions secrets, taken from 1Password:
- `MACOS_CERTIFICATE` (base64 `.p12`), `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY`;
- `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`, `APPLE_API_KEY` (base64 `.p8`);
- the public app configuration `TEAMSFAST_CLIENT_ID`, `TEAMSFAST_TENANT`, `TEAMSFAST_RELAY_URL`, which is baked into the build.

`just release` builds the same bundle locally; it is only signed and notarized when those variables are set.

The relay deploys from this repository on Coolify (Dockerfile build, base directory `/relay`). A GitHub push webhook triggers its deployment.
