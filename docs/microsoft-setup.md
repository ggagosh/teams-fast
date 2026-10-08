# Microsoft 365 setup

TeamsFast targets Microsoft 365 work/school accounts in the public commercial cloud. Teams Personal is outside the supported API path.

## Register an application

1. Create an app registration in Microsoft Entra. For one tenant, select **Accounts in this organizational directory only**.
2. Copy the **Application (client) ID** and **Directory (tenant) ID**. These are public configuration; never put a client secret in the desktop app.
3. Under **Authentication**, add the platform **Mobile and desktop applications** with the redirect URI `http://localhost` (any port is accepted for loopback). Also enable **Allow public client flows**, which the "Sign in with a code instead" fallback needs.
4. Add Microsoft Graph **delegated** permissions: `openid`, `offline_access`, `User.Read`, `User.ReadBasic.All` (profile photos, people search), `Chat.ReadWrite`, `Chat.Create`, and `ChatMessage.Send`. The app requests these in one sign-in. For planned features, also add `Presence.Read.All`, `Presence.ReadWrite`, `Files.Read.All`, and `Files.ReadWrite`.
5. Complete consent according to tenant policy.

## Sign in

Release builds have the client ID, tenant and relay URL built in. The app opens to **Sign in with Microsoft**: the system browser handles sign-in (authorization code with PKCE, redirected to a one-time `http://localhost:<port>` listener), and the chats appear. **Sign in with a code instead** uses the device-code flow, for example when the browser is on another device.

For development, copy `.env.example` to `.env`, fill in `TEAMSFAST_CLIENT_ID` and `TEAMSFAST_TENANT`, then run `just dev`. Without IDs, the sign-in screen shows fields for them.

The refresh token is kept in the macOS Keychain and restores the session on launch. Access tokens stay in memory. **Settings → Account → Disconnect account…** removes the saved sign-in and keeps local drafts for that account. History, drafts and pending sends are encrypted locally with a Keychain-held key. See [local history](local-history.md) for retention and recovery.

## Live updates and notifications

The relay is `https://teamsfast-relay.omedialab.com`, hosted on Coolify and built from `relay/Dockerfile` in this repository (base directory `/relay`). The desktop needs no relay secret: it registers with the user's Microsoft ID token. The relay URL can be changed with `TEAMSFAST_RELAY_URL` or under **Settings → Advanced**.

Relay runtime configuration (Coolify):
- `TEAMSFAST_RELAY_PUBLIC_URL` and `TEAMSFAST_RELAY_BIND`;
- `TEAMSFAST_RELAY_CLIENT_ID` and, optionally, `TEAMSFAST_RELAY_TENANT`, so only ID tokens for this app (and tenant) can register;
- the secret operator key `TEAMSFAST_RELAY_KEY`, for scripts such as `scripts/check_relay.py`.

Restarting the relay discards in-memory queues and registrations; clients re-register and catch up.

Under **Settings → Notifications**, enable **Desktop notifications**, optionally **Message previews**, and leave **Quiet mode** off. Chats can be muted from their header. Notifications need the app running and permission in System Settings → Notifications; development builds need `just app`.

## Chat behavior and verification

- Chat lists have previews, activity ordering, title search, and pagination. New-chat search supports 1:1 and group creation using the shared sign-in scope set.
- Conversations load recent messages and offer **Load older messages**. Refresh follows navigation/manual actions, relay changes, and reconnect catch-up.
- Enter sends; Shift+Enter adds a line break. A pending message appears immediately; its text is committed to encrypted storage before sending, then the composer clears. Failed sends retain their text. If delivery is uncertain, refresh and check before retrying.
- HTML becomes safely formatted text with inline HTTP(S) links. Attachment names appear in small cards that open the conversation in Teams; attachment files are not transferred by the desktop.
- Read markers synchronize with Teams using `Chat.ReadWrite`. Counts from partially cached history remain approximate; another device's reads are reconciled on refresh/reconnect.
- Your own message menu offers edit and delete. Edits save plain text; messages containing images, attachments or quotes must be edited in Teams.

Local demo and protocol tests do not prove tenant consent, live Graph push, or OS delivery. Verify those using an approved signed-in conversation and [the acceptance checklist](initial-milestone.md#acceptance-checks).

Primary references: [authorization code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow), [device-code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code), [list chats](https://learn.microsoft.com/en-us/graph/api/chat-list?view=graph-rest-1.0), [create chats](https://learn.microsoft.com/en-us/graph/api/chat-post?view=graph-rest-1.0), [send messages](https://learn.microsoft.com/en-us/graph/api/chat-post-messages?view=graph-rest-1.0), [message subscriptions](https://learn.microsoft.com/en-us/graph/teams-changenotifications-chatmessage), and [chat subscriptions](https://learn.microsoft.com/en-us/graph/teams-changenotifications-chat).
