# Current scope and next work

## Implemented

- GPUI Kit 0.7.1 components on pinned GPUI Fast 0.1.4; eframe/egui removed.
- Kit titlebar, compact chat rows, gray/blue dark/light themes, virtualized bubbles, inline formatted links, attachment cards, and native Settings pages.
- Dedicated send/incoming/read/subscription workers with shared authentication, per-resource subscription failure cooldowns, and partial live status.
- Content revisions avoid transcript copies and remeasurement when messages have not changed; optional latency tracing reports queue/work durations without message data.
- Enter-to-send, Shift+Enter newline, immediate pending-send display, retained drafts, and visible send errors.
- Existing work/school authentication, OS-keyring session persistence, chat/history pagination, people search, and 1:1/group creation.
- JSON settings with legacy draft/preference import and unchanged credential-store identifiers.
- Metadata relay, Graph subscription renewal, reconnect catch-up, notifications, mute/quiet/preview controls, and local unread indicators.
- Recurring 20/45-second Graph refresh loops removed; user-triggered reads and push/recovery fetches remain.

## Verification and next work

Formatting, workspace/all-target compilation, and Clippy passed for the migration; the later request-lane and bubble-layout revision also passed compilation and Clippy. No tests were added or run. The earlier native GPUI app restored the saved account and rendered Georgian messages. The latest layout, scheduling changes, and rich-message interactions await native/manual QA and measurement. See [development](development.md).

1. Check the actual GPUI app: input, selection, Georgian, history scrolling, Settings, themes, and account restoration.
2. Complete [signed-in acceptance](initial-milestone.md#acceptance-checks), including Graph push, OS notification delivery/click routing, and sleep/reconnect.
3. Measure send/incoming queue time, request time, and long-history behavior. Dedicated lanes and revision-based synchronization are implemented, but their performance impact is not yet measured.
4. Verify Windows/Linux and prepare signed distribution.

One active desktop per account/relay remains the subscription limit. Quitting stops notifications. Local unread indicators do not synchronize Teams server read state.

Calls, meetings, channels, attachment transfer, reactions, message editing, richer group management, multiple accounts/devices, and Teams Personal remain outside this milestone.
