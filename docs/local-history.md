# Local history and optimistic updates

## Storage

The desktop uses `rusqlite` with bundled SQLCipher. A dedicated worker owns the database connection; Keychain access, serialization, queries, pruning and disk writes never run on GPUI's thread. The relay has no database dependency.

Each tenant/client/user account has its own encrypted database under `~/Library/Application Support/TeamsFast/history/`. Debug builds use `history-debug/` and their existing separate Keychain service. Filenames are SHA-256 hashes of the account identity. The directory is owner-only and database files are created with mode 0600.

A random 256-bit key lives in Keychain, not settings or the database directory. Initialization checks that SQLCipher is present, uses in-memory temporary storage, WAL and `synchronous=FULL`. First-use key creation is protected by a file lock. Missing keys, unreadable data and newer schema versions produce an error, never silently replace an existing database. Keep the original database and recover Keychain access rather than deleting it if it contains drafts or unconfirmed sends.

The store contains chat summaries, confirmed messages, read markers, drafts and pending sends. Message bodies, titles, drafts and indexes are encrypted, including database pages in WAL. An unlocked running app still holds decrypted content in memory. Encryption is not protection from someone controlling the unlocked user session. Backups made with accessible Keychain keys can also restore the data.

Legacy drafts migrate out of `settings.json` for an account only after its encrypted import commits. Other accounts migrate when opened. Existing backups and legacy `app.ron` files are not rewritten or securely erased. New drafts go to the encrypted store; settings retain only preferences, IDs and mute choices.

## Loading and retention

A remembered account's cached chat list loads while Microsoft sign-in resumes. Cached conversations remain readable if restoration fails; the app shows **Cached history · reconnect to send**. Sending requires a live account and ready encrypted storage. Disconnect removes remembered sign-in and suppresses automatic cache display, but keeps encrypted drafts/history for a later sign-in to the same account.

The sidebar and conversation switcher omit chats hidden in Teams. After **all pages** of a successful `/me/chats` refresh, cached chats absent from that listing are also omitted. A failed or partial list never establishes that a chat is unavailable. This is visibility reconciliation, not a claim that the user left a chat: expanded membership is limited to 25 entries and is not used to infer membership. Meeting chats are labeled as such; channels are not fetched.

Hidden/unavailable conversations with local drafts or unconfirmed sends remain visible for recovery, without unread badges or alerts. Unavailable conversations cannot send, mutate, or mark messages read. Their encrypted records are retained, not deleted; an authoritative later listing can restore availability. Cached data arriving after a completed listing follows the same visibility decision.

Only the selected conversation's cached messages load initially. The latest three visited conversations keep full in-memory histories; inactive ones retain pending changes/sends and their newest message after cache writes are queued. Graph is authoritative: cached content is stale until refreshed. Revisions and modification timestamps prevent delayed cache/history responses from replacing newer updates.

The cache keeps up to 500 confirmed messages per conversation. Periodic global trimming targets 20,000 confirmed messages or 64 MiB of serialized message data; these are cache targets, not hard limits on total file size. SQLite also needs indexes, free pages and WAL space. Drafts and pending sends are never evicted. Images remain in the existing bounded memory cache; this change does not download them for offline use. Local full-text search is not exposed yet, though SQLCipher supports it.

**Settings → Advanced → Clear downloaded history** removes confirmed message history and previews, keeping drafts and unconfirmed sends. Incoming messages and subsequent navigation can populate the cache again. The operation does not delete anything from Microsoft Teams and does not promise erasure from old backups.

## Sending and updates

Sending immediately shows a local bubble. Before a message POST is permitted, the store commits its text and clears the old draft in one transaction. If storage fails, nothing is posted. A confirmed response replaces the placeholder and its cached copy. On restart, unfinished sends appear as **Send not confirmed**, never automatically replayed. An uncertain send can already exist on Microsoft: refresh/check before deliberately sending again. Its context menu can remove only the local placeholder; this never unsends the server message.

Reactions, edits and deletions are overlays on the latest confirmed message, not modifications to that baseline. One operation per message is allowed at a time. Server pushes keep updating the baseline beneath the overlay. A definite rejection removes the overlay; a transport failure or failed confirmation read retains **Update not confirmed** and requests reconciliation by GET. No mutation POST/PATCH is automatically repeated. Pending edit/reaction overlays are not durable across restart; the next Graph refresh supplies server state. Edit text remains in the edit dialog during the running session so it can be recovered after a rejection.

Own-message actions are in the context menu. Delete asks for confirmation and uses Graph soft delete. Editing saves plain text and explicitly warns that formatting is removed. Messages containing attachments, inline images or quotes must be edited in Teams rather than silently discarding those parts.

## Quit and update restart

The red main-window close button hides TeamsFast and keeps it running for updates and notifications. The window, draft and any open edit dialog stay in memory; clicking the Dock icon or a current notification reactivates it. Hiding cancels queued read intent. Use **⌘Q** to quit.

Normal Quit and Sparkle's requested restart wait for a worker acknowledgement: current drafts, pending sends and pending local-message removals commit transactionally, then preferences are saved. This is a full user-data snapshot, not just a queue-flush marker after best-effort writes. The UI is modal and incoming model changes pause during that save. Failed saves leave the app open; **Keep working** cancels the quit/restart wait. After cancelling or a save failure, **Check for Updates** retries a postponed update restart. An open dialog must be finished or dismissed before quitting, so an edit is not silently discarded.

In-flight sends may still complete at Microsoft while closing. Their text remains recoverable locally as unconfirmed, and is never automatically resent. OS-initiated termination only has GPUI's 200 ms best-effort shutdown hook; forced termination or power loss cannot guarantee flushing the latest unsaved keystrokes.

## Read state

The app reads `chatViewpoint.lastMessageReadDateTime` from chat-list responses. It optimistically clears activity when a loaded conversation is active and scrolled to its bottom, then calls `markChatReadForUser` with delegated `Chat.ReadWrite`. Read requests expire after a short queue/token-refresh wait and are canceled when the user leaves the conversation; they are never persisted or replayed offline.

Only ordinary incoming messages contribute unread activity. Graph system events render as quiet activity rows rather than empty “Teams” bubbles; they never generate badges or alerts. A missing/unknown preview type cannot manufacture an unread count, and old empty cached event bubbles are omitted. See [chat status](chat-status.md) for why **Sent to Teams** does not mean recipient delivery or reading.

Graph's action has no message/timestamp cutoff, so a message arriving while the request is in flight can also become read. Incoming read changes on another device are reconciled on chat-list refresh/reconnect, not guaranteed to push instantly. Graph provides a read marker, not an exact unread count: counts based on partial cached history can undercount older unread messages. The undocumented self chat is excluded from server read synchronization.

The all-chat-message delta endpoint requires application permissions; it is not used by this delegated desktop client. Reconnection uses the existing notifications and targeted catch-up reads, not a complete offline replica of the account. Tenant access revocations and changes while offline may remain visible in cached history until refreshed or cleared.

## Acceptance

Local compilation, existing tests and a synthetic SQLCipher smoke check are separate from live acceptance. Check cached startup, draft restoration, interruption during send, account switching, disk/Keychain failures, reactions rejected by Graph, edits/deletions arriving during refresh, read markers on another device and clearing history with pending sends using an approved account. Demo mode does not access the store or Microsoft. Notification and relay delivery checks remain separate.
