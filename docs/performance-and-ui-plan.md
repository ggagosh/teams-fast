# Faster chat and a GPUI interface

Research: 2026-10-07. This is a proposed implementation plan, not an implemented migration or measured speedup. The user reports that live-update setup now works. This pass inspected local code and current primary sources; it did not send Teams messages or profile the signed-in account.

## Recommendation

Fix scheduling and sending feedback, then replace the custom egui interface with **GPUI Kit** after one representative prototype passes. Evaluate **GPUI Fast** on that same prototype, pinned to its verified release; keep the normal Kit runtime as the fallback. Stop investing in a large custom egui redesign. The component migration addresses interface quality; the network changes address sending and incoming-message latency.

Required composer behavior: **Enter sends; Shift+Enter inserts a newline**. Keep Cmd/Ctrl+Enter as an optional additional shortcut. IME confirmation must not submit; holding Enter must not send repeatedly.

## What is currently slow

These mechanisms are verified in code. Their contribution to this user's observed delay still needs timings.

| Finding | Consequence | First change |
| --- | --- | --- |
| One bounded FIFO queue and one blocking Graph worker handle sending, reads, authentication, and subscription HTTP. Individual requests have a 20-second timeout. [Worker](../src/teams.rs#L150) | A slow history or subscription request can hold up a send or incoming-message fetch. The timeout is a ceiling, not a measurement of normal latency. | Give sends and visible-chat reads priority; allow bounded concurrent I/O with background subscription work independent of interactive work. |
| A live send only sets `chat.sending`; the timeline gets the message after the POST response. [Send](../src/app.rs#L826), [response](../src/teams.rs#L393) | Every network delay is visible as an apparently unresponsive send. | Show a pending message immediately, preserve its text, then reconcile using the returned server ID. |
| `change_inflight` permits one changed-message fetch across the whole app, and pending IDs are ordered lexicographically. [Dispatch](../src/app.rs#L710) | Activity in background chats can delay the visible chat during a burst. | Prioritize the selected chat, coalesce duplicate IDs including those already in flight, and use a small quota-aware read concurrency limit. |
| Subscription work runs only after a one-second command-queue timeout, one operation per pass. Startup removes prior same-relay subscriptions before recreating them. [Scheduling](../src/teams.rs#L173), [leases](../src/teams.rs#L636) | Startup cost grows with chat count. Busy traffic can postpone maintenance; subscription calls can subsequently block interactive work. | Schedule maintenance by deadlines; reuse valid leases and stable callback identity where possible, renew before expiry, and prioritize the active chat. |
| The selected chat refreshes every 20 seconds and chat summaries every 45 seconds. [Timers](../src/app.rs#L726) | Extra reads compete with sends and may disguise missing push coverage. | Make updates webhook-driven; refresh on user navigation/refresh and bounded recovery events instead of using tighter perpetual polling. |
| Timeline rendering hashes all loaded message text and recalculates grouping/date layout on every render before skipping cached offscreen rows. [Timeline](../src/ui.rs#L545) | Long history can cost CPU even when only a few rows are visible. | Cache presentation by message revision and use Kit's variable-height message scroller in the new UI. Measure before adding another cache. |

The relay's 25-second wait is a heartbeat timeout: a webhook wakes a pending request immediately. It is not a 25-second delivery interval. [Relay](../relay/src/server.rs#L184), [desktop](../src/realtime.rs#L141). Earlier session verification recorded a 0.12-second synthetic webhook-to-waiting-client round trip; it excluded Microsoft's event production and the subsequent Graph message fetch. [Recorded check](development.md#verification-record).

## Microsoft imposes part of the delay

Microsoft lists `chatMessage` change notifications at **under 10 seconds average and one minute maximum latency**. Those published expectations are not a promise of instant delivery or a measured result for this account. We can remove our queueing and rendering delays; changing UI frameworks cannot accelerate Microsoft's notification generation. [Notification latency](https://learn.microsoft.com/en-us/graph/change-notifications-overview#latency).

Send returns a `201` response containing the new message. The current client already uses that object; keep this path rather than adding a confirmation GET. Microsoft documents send limits of **1 request/second per user and per chat**, with additional app/tenant limits. Concurrency must respect those limits and `Retry-After`; it is not permission to issue an unlimited burst. [Send API](https://learn.microsoft.com/en-us/graph/api/chat-post-messages?view=graph-rest-1.0), [Teams throttling](https://learn.microsoft.com/en-us/graph/throttling-limits#microsoft-teams-service-limits).

The official Graph documentation source restricts repeatedly polling for changes and directs clients toward subscriptions; it distinguishes user-triggered reads from recurring polling. Its source includes a once-per-day polling restriction, although the rendered Learn overview checked during this research does not include that section. The existing 20/45-second loops therefore need review and replacement, not acceleration. [Official source: Teams API overview](https://github.com/microsoftgraph/microsoft-graph-docs-contrib/blob/main/api-reference/v1.0/resources/teams-api-overview.md).

Two later options, if measurements justify them:

- **Encrypted notifications containing message data** can remove the fetch after a webhook. That requires certificate rotation, payload validation/decryption, and forwarding opaque encrypted data through the relay. It does not shorten Microsoft's own notification latency. Now implemented: see [architecture](architecture.md#live-updates-and-notifications). [Rich notifications](https://learn.microsoft.com/en-us/graph/change-notifications-with-resource-data).
- **One user-level message subscription** can reduce per-chat setup overhead. Microsoft documents delegated `Chat.Read` support for `/users/{id}/chats/getAllMessages`, but the example uses the **beta subscription endpoint**. Evaluate separately; do not silently switch the whole client to beta or request broader application permissions. [User-level subscriptions](https://learn.microsoft.com/en-us/graph/teams-changenotifications-chatmessage#subscribe-to-changes-at-the-user-level).

Replacing the existing long poll with WebSockets is not the first step: it leaves both the Graph delivery delay and the desktop request queue intact.

## Backend implementation

Use reqwest's asynchronous client with a bounded Tokio executor; Tokio is already present transitively. Make its use explicit and keep runtime ownership in the network layer. Do not run blocking HTTP on GPUI's UI executor.

Keep one authentication/refresh coordinator, but do not hold a global session lock across every HTTP call. Separate interactive dispatch from maintenance, with endpoint/user/chat rate budgets. Cancel obsolete read work after changing accounts or chats, retain generation checks, and avoid starvation of subscription renewals. Start with a small concurrency limit; increase only when timings and throttling behavior justify it.

For sending, introduce a small account-scoped pending outbox: local operation ID, chat, text, and queued/sending/confirmed/unconfirmed/failed state. Persist text before clearing the composer so crashes do not discard it. Replace the pending row with the returned Graph ID and merge duplicate webhook delivery by that ID. Preserve newer composer text. A timeout can mean the server accepted the message: keep an explicit unconfirmed state and never automatically repeat that POST. A request-tracing ID is not an idempotency guarantee.

Retain the existing Graph DTOs, HTML-to-text conversion, account-scoped drafts, safe links, keyring identifiers, relay wire types, and notification routing. Replace the `egui::Context` wake-up dependency in the workers with a small UI wake-up callback; migrate `eframe::Storage` at the settings boundary. A UI replacement should not rewrite authentication or change where credentials live.

## Interface direction

Use Kit's normal components and typography first, with a compact native desktop composition:

- Unified macOS title bar, restrained conversation header, and a narrower chat sidebar with properly elided names/previews.
- A message timeline with consistent sender grouping, readable line lengths, stable history position, and a clear unread marker. Keep the bottom visible while typing; follow new messages only when already at the bottom.
- A compact auto-growing composer, Enter/Shift+Enter behavior, and pending/error status attached to the relevant message.
- A normal settings window with **Account**, **Notifications**, **Appearance**, and **Advanced connection** pages. Put relay URLs, keys, resource counts, and diagnostics under Advanced. Normal operation should show a concise connection state with details available when needed.
- Native shortcuts and focus movement, selectable/copyable text, predictable context menus, Georgian/emoji support, and screen-reader semantics.

GPUI Kit already supplies `Textarea`, `Message`, `MessageGroup`, `MessageScroller`, `Settings`, `TitleBar`, and theme tokens. Application logic still owns grouping, delivery states, preferences, and unread semantics. Current verified versions, exact Fast compatibility patches, platform requirements, and primary sources are in [GPUI research](gpui-research.md).

**Do not assume GPUI Fast means faster compilation.** Its published gains concern release-build rendering workloads. Fast 0.1.0 is explicitly experimental; compare it with Kit 0.7.1 on identical app data and the same machine. No local Kit/Fast build or benchmark was run in this research pass. [Fast release](https://github.com/longbridge/gpui-fast/releases/tag/v0.1.0), [first-party rendering measurements](https://github.com/longbridge/gpui-fast/blob/v0.1.0/README.md).

## Order and acceptance

1. **Measure and fix sending:** time enqueue → dispatch → response → UI application, change Enter behavior, add safe pending feedback, and remove background-request blocking. Record operation types and ephemeral trace IDs, never tokens or message bodies.
2. **Fix live scheduling:** prioritize active-chat events, coalesce bursts, separate subscription deadlines, and replace continuous refresh timers with deliberate recovery. Measure webhook arrival → client receipt → Graph fetch → displayed message separately from Microsoft's event delay.
3. **Prototype one complete GPUI screen:** sidebar, realistic long history, composer, Settings, native title bar. Compare normal Kit with pinned Fast. Use synthetic data; preserve the working client until behavior and development-loop results pass.
4. **Replace the UI:** port account/creation dialogs and notification interaction, preserve settings/keyring/drafts, remove eframe and obsolete widgets/tests, and retain one desktop package plus the existing relay package. Do not maintain two production frontends.

Proposed acceptance targets, not measured claims: local pending feedback within 100 ms; no interactive send waiting behind an unrelated background HTTP request; stable scrolling within the display's frame budget; no lost or duplicated sends, drafts, or notifications under retry/reconnect fixtures. API rate limits and Microsoft response time remain explicit exceptions to dispatch/confirmation targets.

Record p50/p95 timings and sample counts using delayed mock HTTP plus an approved signed-in test conversation. Compare frame time, idle CPU/RSS and three representative warm edit→check/build/test cycles against today's egui baseline. Measure a cold build in a separate target directory; do not destroy the working cache. Keep Fast only if it demonstrates useful improvement without input, accessibility, rendering, or build-loop regressions.

This plan changes no application code, dependencies, deployment, or keyboard behavior yet.
