<p align="center"><img src="assets/icon/logo-256.png" width="128" alt="TeamsFast icon"></p>

<h1 align="center">TeamsFast</h1>

<p align="center">A small, fast, native Microsoft Teams chat client for macOS.<br>Rust + <a href="https://gpui-kit.com">GPUI Kit</a> on <a href="https://github.com/longbridge/gpui-fast">GPUI Fast</a>. No Electron, no WebView.</p>

## Why

The official Teams app runs a web app in a WebView. On a typical Mac it uses about **2.3 GB** of memory across a dozen processes. TeamsFast is one native process and uses about a tenth of that. New messages appear in about **half a second**.

## Install

1. Download `TeamsFast-<version>-macos-arm64.zip` (Apple Silicon) or `…-x86_64.zip` (Intel) from [Releases](https://github.com/ggagosh/teams-fast/releases).
2. Unzip it and move `TeamsFast.app` to **Applications**.
3. Open it and choose **Sign in with Microsoft**. Your browser handles the sign-in, then your chats appear.

Releases are signed with Developer ID and notarized by Apple. To get alerts and the Dock badge, allow TeamsFast in **System Settings → Notifications**.

Requirements: macOS 11 or later and a Microsoft 365 work or school account in a tenant where the TeamsFast app registration is allowed. Teams Personal isn't supported by Microsoft's API.

## Features

- Chats with search, unread counts, mute, and a Dock badge
- Message history with formatting, inline images, link previews, reactions, and attachment cards
- Instant sending (Enter sends, Shift+Enter adds a line); drafts are kept per chat
- New 1:1 and group chats
- Live updates through encrypted Graph change notifications, decrypted on your Mac
- Native notifications that open the chat when clicked
- Light and dark appearance, Georgian and emoji text, a unified macOS window

Not included: calls, meetings, channels, sending files, and editing or deleting messages.

## How it works

- **Microsoft Graph, directly.** The app talks to Graph with your delegated permissions (`Chat.Read`, `ChatMessage.Send`). Tokens live in memory and the macOS Keychain only.
- **A tiny relay for push.** Graph delivers change notifications to a webhook, and a desktop app can't receive webhooks. [`relay/`](relay) is a small Rust server that keeps a short in-memory queue and holds a long-poll open to the app. Graph encrypts each message with a key that only your Mac has, so the relay can't read message content. To register, the app presents your Microsoft ID token, which can't access Teams.
- **Memory.** Images are decoded once at display size and kept within a fixed budget.

See [architecture](docs/architecture.md) for details.

## Development

```sh
cp .env.example .env   # public client/tenant IDs and the relay URL
just dev               # run (signed with your Apple Development identity for stable Keychain access)
just app               # run as a .app bundle (needed for notifications and the Dock badge)
just dev --demo        # sample data, no account
just check             # formatting, compilation, Clippy
```

Setting up your own Entra app registration and relay: [setup](docs/microsoft-setup.md). Workflow, CI, and releases: [development](docs/development.md). UI conventions: [DESIGN.md](DESIGN.md).

To release, bump `version` in `Cargo.toml`, commit, and push a matching `vX.Y.Z` tag. GitHub Actions builds, signs, notarizes, and publishes both architectures.
