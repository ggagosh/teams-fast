# Architecture

The desktop is one Rust package using GPUI Kit 0.7.1 and GPUI Fast 0.1.4. Fast is pinned to `edbd7641e9cc5afae84c542e04a13b3171489875`; its compatibility crates satisfy Kit's 0.3.8 snapshot requirement. eframe/egui has been removed. The separate `relay/` workspace package shares protocol types without enabling server dependencies in desktop builds.

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | GPUI initialization, fonts, window, menus, shortcuts, demo flags |
| `src/app.rs` | GPUI view/controller, input entities, transcript synchronization, event tasks |
| `src/ui.rs` | Kit titlebar, sidebar, message scroller, composer, shared visuals |
| `src/dialogs.rs` | Connection, new-chat, and disconnect dialogs |
| `src/preferences.rs` | Native Settings window: Account, Notifications, Appearance, Advanced |
| `src/state.rs` | UI-independent chat/session state and worker event application |
| `src/model.rs` | Messages, chats, merging, ordering, reactions, demo data, notification deduplication |
| `src/html.rs` | Teams HTML tag rewriting, image/link extraction, link-preview metadata parsing |
| `src/teams.rs` | Entra/Graph HTTP, shared authentication, chat creation, request lanes, subscriptions |
| `src/realtime.rs` | Relay registration, long polling, reconnect/backoff |
| `src/settings.rs` | JSON preferences/drafts, legacy import, OS credential-store access |
| `src/notifications.rs` | Native notifications and click events |
| `relay/src/lib.rs`, `relay/src/server.rs` | Shared wire types and the independently deployed relay |

## Rendering and background work

GPUI retains the view and input entities. A bounded async wake channel notifies the view when the existing workers have events; the UI drains them and synchronizes affected state. HTTP stays off the UI thread. The small UI timer handles deadlines and saving dirty settings; it does not periodically fetch Graph chats/messages.

Kit's `MessageScroller` owns variable-height virtualization. Each chat has a content revision; unchanged messages do not trigger transcript copies or remeasurement. Changed transcripts splice ID ranges and remeasure rows whose message content differs. `MessageGroup`, `Message`, and `Bubble` compose neutral incoming and right-aligned tinted own messages.

Graph work is separated into a bounded send queue (16), incoming-message queue (32), normal-read queue (32), and an independent subscription worker. Each request lane remains serial within itself. Cloned sessions share authentication state; token refresh is coordinated under its lock, while message/history/subscription HTTP runs outside that lock. A slow read or subscription request no longer occupies the send worker. Session generations continue rejecting stale work after sign-in changes or disconnect.

Credential saves and deletion share a separate gate, and revoked sessions cannot save after sign-out. History responses carry the chat revision from request time; message revisions and Graph modification timestamps keep delayed history from replacing newer edits or deletion events, including deletions received before a message is loaded. Transcript splices invalidate the retained boundary row so sender/day grouping keeps the correct height.

The send lane spaces request starts by one second (Microsoft's per-chat send budget); the incoming-message lane has no spacing. Both apply `Retry-After` cooldowns. Subscription maintenance has its own pacing. Message POSTs are never automatically retried. These are implementation changes, not measured end-to-end speedup claims.

HTML bodies render through Kit `TextView::html` (bold, italic, underline, strikethrough, lists, quotes, code, tables, links, images); remote HTML is never executed. `src/html.rs` first rewrites Teams-only tags: `<emoji>` becomes its character, `<at>` mentions become bold, and inline `<attachment>` placeholders are dropped. `html2text` still produces the plain text used for previews, notifications, and copying. Plain-text bodies keep the escaped-Markdown path.

Images, link previews, and (disabled) profile photos load on a three-thread media pool separate from the send/read lanes. Graph-hosted images (`hostedContents`) are fetched with the access token, which is only ever sent to `graph.microsoft.com`; other https images and pages use a separate credential-free client with limited redirects and size caps (8 MiB images, 512 KiB pages). Each URL is requested once per run; failures show nothing and are not retried. Only the open conversation's media is requested. Link previews read the first https link's Open Graph/Twitter/`<title>` metadata directly from the site, so the site sees the request; **Settings → Appearance → Link previews** turns them off. Graph exposes no Teams unfurl data for chats.

The self chat ("Name (You)", Graph ID `48:notes`) is undocumented and absent from `/me/chats`. Graph rejects `GET chats/48:notes` ("not a ChatThread", observed 2026-10-08) but serves `chats/48:notes/messages`, so after the last page of a full chat load the app reads its newest message and builds the entry from it, titled with the account name and pinned first. It is not added to live-update subscriptions. Graph also omits some members (14 of one account's 264 chats listed only the signed-in user); such 1:1 chats take their title from the latest sender when that is not the user.

Replies (`messageReference` attachments) render as a quote above the text. SharePoint/OneDrive `reference` attachments render as Kit `Attachment` cards that open the file's web address; other attachments (cards) open the conversation in Teams. The desktop does not download files.

Reactions come from Graph `reactions`, grouped per type, and render as Kit `BubbleReactions` chips. Clicking a chip or choosing a reaction in the message's context menu toggles it immediately, then calls `setReaction`/`unsetReaction` (delegated `ChatMessage.Send`) on the send lane and reloads that message. Legacy names (`like`, `heart`, …) map to emoji; removal sends the original `reactionType`.

## Sending and storage

Enter submits and Shift+Enter adds a newline. The message appears in the timeline immediately (dimmed, "Sending…") and the composer clears, so several messages can be queued; they are sent in order. Microsoft's response replaces the local placeholder. On failure or timeout the bubble is marked "Not confirmed" and its text is restored to an empty composer; a timeout can still mean delivery, so nothing is resent automatically. The in-flight text is not persisted, so quitting during a send can lose it. Message POSTs are never automatically repeated. Messages merge by Graph ID and sort using parsed RFC 3339 timestamps.

Refresh tokens and the rich-notification key live only in the macOS Keychain (service `dev.teamsfast.desktop`). Debug builds are signed with the developer's Apple Development identity and a designated requirement on identifier and team, so rebuilt binaries keep Keychain access without prompts. Secrets from the earlier debug-only `dev-secrets.json` move into the Keychain on first read, and the file is deleted once empty. Access tokens stay in worker memory. Public settings, appearance, notification preferences, account-scoped drafts, and mutes are saved to `settings.json` via a temporary file and rename. If that file is absent, the app imports `settings-v1` from legacy `app.ron`. On macOS these files live under `~/Library/Application Support/TeamsFast/`. Draft data is local plaintext application data; message history is not persisted. Explicit demo runs do not load/save account settings.

## Live updates and notifications

The desktop subscribes to `/users/{id}/chats/getAllMessages` (every chat the user is in, delegated `Chat.Read`) and `/users/{id}/chats`, so live updates cover all chats within seconds of sign-in, including chats created later. Per-chat `/chats/{id}/messages` subscriptions are created only while the user-level subscription is rejected. New and renewed subscriptions take priority over deleting the previous run's subscriptions, which happens one per second in the background. Metadata-only subscriptions last 50 minutes and renew before expiry. A resource failure gets its own retry deadline so other resources can progress; throttling can delay the subscription lane. Existing resource paths are normalized before same-relay cleanup. The relay never receives Graph access tokens or readable message bodies. To register, the desktop presents the signed-in user's Microsoft ID token (`openid` scope; it cannot call Graph). The relay checks it against Microsoft's signing keys, this app's client ID (`TEAMSFAST_RELAY_CLIENT_ID`), and optionally the tenant (`TEAMSFAST_RELAY_TENANT`). It then returns a per-registration key that the desktop uses for polling and unregistering. The operator key (`TEAMSFAST_RELAY_KEY`) remains for scripts.

Message subscriptions are rich notifications (`includeResourceData`). `src/rich.rs` creates a 2048-bit RSA key once per installation and stores it with the other secrets. In release builds that is the OS credential store; in debug builds it is `dev-secrets.json`. Each launch presents a fresh self-signed certificate for that key; its `encryptionCertificateId` is derived from the public key. The relay forwards Graph's `encryptedContent` and the batch's `validationTokens` unchanged. It drops content over 48 KB or tokens over 16 KB, and it cannot decrypt anything. Before using the content, the desktop checks every validation token:

- the RS256 signature verifies against Microsoft's published signing keys, which are cached and refetched on an unknown key ID at most once a minute;
- the token is unexpired, allowing five minutes of clock skew;
- the audience is this app's client ID;
- the caller is Graph change tracking (`0bf30f3b-4a52-48df-9a82-234910c4a086`);
- the issuer is Microsoft.

It then unwraps the AES key (RSA-OAEP/SHA-1), checks the HMAC-SHA256, and decrypts (AES-256-CBC). Any failure falls back to fetching the message by ID, as do notifications without content. If Graph rejects a rich subscription, the session uses ID-only subscriptions instead. Deleted messages carry no content.

Relay long polls wait up to 25 seconds and wake immediately on valid webhooks. Registration/polling requires the operator key; webhook `clientState` is validated. Bounded queues and registrations are in memory. Relay resets, reconnects, and lifecycle signals cause catch-up. Initial/user-triggered reads remain; the previous 20-second selected-chat and 45-second chat-list polling loops are removed.

Only one active desktop per account/relay is supported because startup replaces prior same-relay subscriptions. Relay connectivity and Graph subscription progress/errors remain separate in Settings. Partial progress appears as `Live · N of T chats`; it is not a claim that every resource is subscribed.

Notifications suppress own messages, duplicates, initial history, muted chats, quiet mode, and activity already being read. Click events carry the session generation and chat ID. The app must remain running; fully quitting stops updates and notifications. macOS needs the native bundle (`just app`) and OS permission. The bundle asks for alert, sound and badge permission at launch. The Dock badge shows the unread count of chats that aren't muted. macOS hides it unless TeamsFast notifications (with badges) are allowed in System Settings. Other platforms have no badge.

## Boundaries

Graph requests retain timeouts, response-size limits, origin-checked pagination, disabled redirects, and throttling handling. The public commercial Microsoft cloud is the current target. Client unread indicators do not synchronize Teams server read state.

Kit supplies the component/theme system; Fast supplies the runtime. Noto Sans Georgian is registered alongside native font handling. Georgian messages rendered in the native launch; input and broader glyph coverage remain manual acceptance checks. Existing backend tests remain, but no tests were added or run for this migration and the old egui UI tests were removed.
