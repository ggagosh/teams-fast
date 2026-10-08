# TeamsFast

<!-- impeccable:product-schema 1 -->

## Platform

Native desktop, developed on macOS. Windows and Linux native behavior remains unverified.

## Stack

Rust with GPUI Kit 0.7.1 and GPUI Fast 0.1.4, explicitly selected by the user for the full migration. Use Kit components and standard library primitives first. Keep the desktop in one Cargo package, with the separately deployed relay as the only additional workspace package.

## Users and purpose

Microsoft 365 work/school users who want a fast desktop client for 1:1 and group chats. The owner is a tenant administrator with an Entra registration and has reported basic client and live-update setup working. Those reports do not replace a recorded end-to-end acceptance run.

## Confirmed requirements

- Use a coherent native interface and reusable GPUI Kit controls.
- Enter sends; Shift+Enter adds a newline.
- Georgian text must work in message display and input.
- Receive incoming changes through the approved metadata-only HTTPS relay.
- Preserve drafts and never automatically repeat message POSTs.
- Store credentials in the OS credential store.
- Use formatting, compilation, Clippy, and the runnable app for this migration. Do not add new tests; the user will check the native app.
- Keep product and development documentation in `docs/`.

## Current operating model

The GPUI interface uses Kit components with gray/blue semantic themes, a compact conversation list, virtualized message bubbles, inline formatted links, attachment cards, and a compact composer. Incoming messages use neutral bubbles; own messages align right with a tint. Pending sends appear immediately. Account, Notifications, Appearance, and Advanced remain separate Settings pages.

Microsoft integration retains device-code sign-in, token refresh, optional OS-keyring persistence, paginated chats/history, people search, and 1:1/group creation. Drafts and public preferences use local JSON storage, importing the previous application settings when needed. Sends, incoming-message fetches, normal reads, and subscription maintenance now have separate workers; authentication state and token refresh are shared. Message revisions avoid copying and remeasuring unchanged transcripts.

The relay at `https://teamsfast-relay.omedialab.com` is deployed through Coolify. The desktop manages subscriptions and performs reconnect catch-up; recurring 20/45-second Graph refresh loops have been removed. Message content stays on the desktop-to-Graph path.

One active desktop per account on the same relay is supported. Notifications require a running app; macOS delivery requires its native bundle and OS permission.

## Remaining validation and scope

- Check the migrated runnable app, including Georgian, keyboard input, history scrolling, themes, and Settings.
- Record signed-in Graph delivery, subscription renewal/recovery, credential restoration, migrated drafts, and OS notification delivery/click routing.
- Measure queue time, request time, and native responsiveness before claiming a speedup from the new worker layout or transcript changes.
- Verify Windows/Linux and define release packaging/signing.
- Keep calls, meetings, channels, attachment transfer, multiple accounts/devices, and Teams Personal outside this milestone.

[DESIGN.md](DESIGN.md) records the current component system. See [the milestone](docs/initial-milestone.md) for acceptance checks.
