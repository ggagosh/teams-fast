# Chat activity and send status

TeamsFast uses delegated Microsoft Graph, not the official Teams client's private protocols. The following reflects the documented v1.0 and beta surfaces reviewed for this implementation.

## Outgoing messages

- **Sending…**: the local message is being saved or its POST is in flight.
- **Sent to Teams**: Graph confirmed message creation, or the message was retrieved from Teams. It does **not** confirm delivery to another device, notification delivery, or reading by a recipient.
- **Send not confirmed**: a local send has no confirmed result. Microsoft may already have accepted it. Keep its text, check the conversation, and retry only deliberately; TeamsFast never automatically repeats a message POST.

[Send chatMessage](https://learn.microsoft.com/en-us/graph/api/chat-post-messages?view=graph-rest-1.0) returns `201 Created` and the created message. The [beta method](https://learn.microsoft.com/en-us/graph/api/chat-post-messages?view=graph-rest-beta) has the same response contract.

No supported per-recipient delivery/read receipt interface was found in the documented [v1.0 chatMessage](https://learn.microsoft.com/en-us/graph/api/resources/chatmessage?view=graph-rest-1.0) or [beta chatMessage](https://learn.microsoft.com/en-us/graph/api/resources/chatmessage?view=graph-rest-beta) resource/method surfaces. TeamsFast therefore does not show Delivered, Read, Seen, or checkmarks implying those states. This is a conclusion about the current documented API, not a claim about all future Microsoft APIs.

## Typing

Although the `messageType` enum lists `typing`, Microsoft's [Teams messaging overview](https://learn.microsoft.com/en-us/graph/teams-messaging-overview) explicitly says `typing` and `chatEvent` are not currently in use. An enum member alone is not a supported typing-send endpoint or receive contract. TeamsFast does not fabricate typing indicators.

[ACS Chat](https://learn.microsoft.com/en-us/azure/communication-services/quickstarts/chat/get-started) has typing/read receipt methods, but those are a different service. [Teams bot/agent read receipts](https://learn.microsoft.com/en-us/microsoftteams/platform/bots/how-to/conversations/conversation-basics) apply to user-to-agent personal chats with application RSC permission, not this delegated user chat client. Neither supplies a shortcut to these features here.

## My unread activity and visible chats

[`chatViewpoint.lastMessageReadDateTime`](https://learn.microsoft.com/en-us/graph/api/resources/chatviewpoint?view=graph-rest-1.0) is the **current user's** read-through timestamp, not the other participant's receipt. Counts computed from partially loaded history are approximate. Only ordinary incoming messages contribute unread activity; system events do not. Other-device reads currently reconcile on chat-list refresh/reconnect, although Microsoft also documents [user-specific chat change notifications](https://learn.microsoft.com/en-us/graph/teams-changenotifications-chat).

The same viewpoint provides `isHidden`. TeamsFast honors it and reconciles cached chats only after a complete successful list refresh. [`/me/chats`](https://learn.microsoft.com/en-us/graph/api/chat-list?view=graph-rest-1.0) returns chats the caller belongs to, including meeting chats; it is not a channel listing. Expanded members are capped at 25, so missing the caller from that expansion does not establish non-membership. Local drafts and unconfirmed sends are kept even if a chat becomes hidden or unavailable. See [local history](local-history.md).

### Old call-only meetings

The default sidebar and empty-query ⌘K list omit meeting threads whose latest known activity is at least **30 days old**, but only after a complete successful Graph history scan finds no non-system messages. A last-message preview containing a call event alone is not sufficient: earlier pages may contain real conversation. Unknown dates, unknown message types without event details, failed scans and incomplete scans stay visible. Checks run one page at a time on the existing background worker and do not load the transcript, mark messages read, or change Teams visibility.

This is a **TeamsFast list policy**, not a claim to reproduce Microsoft's undocumented default-list rules. Regular chats, meetings with actual messages, recent meeting activity, drafts and pending sends remain listed. Searching by name in the sidebar or ⌘K includes the filtered old call-only meetings; opening one keeps it selected even after the search is cleared. Server-hidden/unavailable chats retain their existing restrictions.

Graph masks `systemEventMessage` as `unknownFutureValue` unless requests include `Prefer: include-unknown-enum-members` ([messageType contract](https://learn.microsoft.com/en-us/graph/api/resources/chatmessage?view=graph-rest-1.0)). TeamsFast requests those enum members and also recognizes structured `eventDetail` as system activity, including in push payloads. The transcript and history inspector share that classification.

Classification is cached with encrypted chat summaries. The incorrect `has_messages` evidence written by 0.5.1 is ignored and recomputed under `has_chat_messages`; existing messages, drafts, keys and other settings are untouched. New preview/activity metadata invalidates a negative result; observing any actual message keeps the thread listed even after cached history is trimmed or cleared. A failed scan leaves the chat listed and can be retried with Refresh. No data is deleted and no message POST is repeated.

These findings do not constitute live Graph, relay or macOS notification acceptance.
