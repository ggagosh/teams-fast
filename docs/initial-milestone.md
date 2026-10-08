# Initial daily-use milestone

Status on 2026-10-07: the full GPUI Kit/Fast migration is implemented and passes formatting, compilation, and Clippy. Native and signed-in acceptance remain separate. The user superseded the earlier prototype gate and requested no new tests.

## Outcome

Leave TeamsFast running, notice an incoming message, open its conversation, read history, and reply without manual refresh or losing a draft or scroll position.

## Implemented scope

| Area | Current implementation |
| --- | --- |
| Interface | Kit titlebar, compact sidebar, neutral/tinted message bubbles, compact composer, native Settings; see [DESIGN.md](../DESIGN.md) |
| Conversations | Previews, activity ordering, title search, local unread counts |
| New chats | People search and 1:1/group creation with additional delegated consent |
| Reading | Kit variable-height scroller, content revisions, older pages, inline formatted links, attachment cards |
| Sending | Enter sends; Shift+Enter adds a newline; immediate pending text, retained drafts, no automatic POST retries |
| Scheduling | Separate bounded send/incoming/read queues and subscription worker; shared authentication |
| Persistence | OS-keyring credentials; local JSON preferences/drafts/mutes, importing legacy settings |
| Incoming activity | Per-chat message and user-chat subscriptions, metadata relay, direct Graph fetches |
| Recovery | Subscription renewal with per-resource failure cooldowns, partial live progress, reconnect catch-up, duplicate suppression; no recurring Graph polling |
| Notifications | Native delivery/click integration, mute/quiet controls, optional previews, suppression rules |

The relay remains deployed through Coolify at `https://teamsfast-relay.omedialab.com`. It authenticates clients, validates webhook state, bounds metadata queues, and immediately wakes long polls. Microsoft tokens and message bodies remain outside the relay.

## Operating limits

- Notifications require a running app and, on macOS, the native bundle and OS permission. Fully quitting stops them.
- Only one active desktop per account on the same relay is supported; new sessions replace prior same-relay subscriptions.
- Each request lane remains serial; token refresh and Microsoft service latency can still delay work. Dedicated lanes and pending display do not establish a measured speedup.
- Client unread counts do not synchronize Teams server read state.
- Calls, meetings, channels, attachment transfer, reactions, and sending edits/deletions are outside this milestone.

## Acceptance checks

The user will check the runnable native app. Record local compilation, native interaction, relay protocol, and signed-in Graph/OS results separately.

- Enter sends once; Shift+Enter creates a newline; IME confirmation does not submit.
- Drafts survive switching chats, pending/failing sends, settings migration, and restart; successful acknowledgement preserves newer text.
- Selection/copying, Georgian, emoji, long messages, safe links, keyboard navigation, both themes, and small windows work.
- Older-history prepending and resizing preserve a usable reading position; the composer remains available.
- Settings pages, sign-in restoration, new-chat consent/creation, and credential forgetting work.
- Another Teams client causes an automatic incoming message through the deployed relay.
- A background incoming message produces one notification; clicking it opens the correct conversation.
- Own sends, duplicate events, initial history, and old catch-up traffic do not flood notifications; mute/quiet/preview settings work.
- Sleep/offline/reconnect catches up without duplicates, and subscription renewal/new-chat discovery remain working.

See [development](development.md#verification-record) for the current evidence. Earlier egui test results and screenshots are historical, not GPUI acceptance.
