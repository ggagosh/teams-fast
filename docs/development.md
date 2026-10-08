# Development and checks

Use Rust 1.95 or newer and `just`. Keep `Cargo.lock` and the pinned GPUI Fast revision.

```sh
just dev
just dev --demo
just dev --demo --compact
just check
```

Debug builds compile dependencies with `opt-level = 3`, because GPUI is unusably slow unoptimized. `cargo run` and `just app` sign the debug build with your Apple Development identity (`scripts/sign_dev.sh`, through the Cargo runner in `.cargo/config.toml`). Keychain access therefore survives rebuilds; set `TEAMSFAST_DEV_SIGNING_IDENTITY` to choose another identity. The sign-in and the rich-notification key live in the Keychain under `dev.teamsfast.desktop`.

`just dev` loads the ignored `.env`; copy `.env.example` for a new checkout. It holds only public IDs and the relay URL. Release builds bake the same values in at compile time from the CI variables, so the app opens straight to **Sign in with Microsoft**. Plain Cargo uses inherited environment variables. Demo mode ignores saved account settings and does not persist its changes. Compact mode opens at 780 × 580 logical pixels; the minimum window is 760 × 520.

`just check` runs formatting, workspace/all-target compilation, and Clippy with warnings denied. Per the user's migration instructions, do not add new tests. Existing backend tests remain, but they were not run in this migration; egui UI tests were removed with the old frontend. Native interaction is checked in the runnable app.

The desktop is the default workspace member, so normal desktop builds do not compile the relay server. GPUI Kit is pinned to 0.7.1; all patched Fast compatibility crates use the same revision from `Cargo.toml`.

## Native macOS app

```sh
just app          # or: just app --demo
```

This builds `target/TeamsFast.app`, signs it ad hoc, and runs it from the terminal. `.env` and the trace output work as with `just dev`. OS notifications and the Dock badge need this bundle; `just dev` cannot show them. The first launch asks for notification permission. If it was denied before, enable TeamsFast (including badges) in System Settings → Notifications. Omit `--demo` to restore a remembered account. Launching the bundle with `open` does not load `.env`; the current local bundle can include public IDs and the relay URL in its generated `LSEnvironment`, or configuration can be entered in the app. Never embed keys or tokens in the bundle. Signing/notarization remain release work.

Check the runnable app:

1. Resize the sidebar and window; check the Kit titlebar, transcript, and composer.
2. Switch chats with drafts, send a demo message using Enter, and add multiline text with Shift+Enter.
3. Check Georgian, emoji, long content, selection/copying, inline links, attachment cards, and keyboard focus.
4. Open Settings with Cmd+, and check Account, Notifications, Appearance, and Advanced, including the light/dark switch.
5. Check older-history prepending, scrolling, pending sends, and visible errors.
6. In the bundled app, use **Send test notification** and check OS permission, delivery, and click routing separately.
7. Complete the signed-in checks in [the milestone](initial-milestone.md) using an approved conversation.

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
