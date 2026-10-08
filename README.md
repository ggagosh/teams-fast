# TeamsFast

A fast native Microsoft Teams chat client for macOS (Microsoft 365 work/school accounts), built with Rust, GPUI Kit, and GPUI Fast.

**Install:** download the latest `TeamsFast-*-macos-*.zip` from [Releases](https://github.com/ggagosh/teams-fast/releases), unzip, move `TeamsFast.app` to Applications, open it and choose **Sign in with Microsoft**. Releases are signed and notarized.

Features: chat list with search and unread counts, message history, sending (Enter sends, Shift+Enter adds a line), reactions, link previews and images, new chats, live updates within about half a second through rich (encrypted) Graph notifications, native notifications, and a Dock badge.

## Development

```sh
cp .env.example .env   # public client/tenant IDs and the relay URL
just dev               # run (signed with your Apple Development identity for Keychain access)
just app               # run as a .app bundle (needed for notifications and the Dock badge)
just dev --demo        # sample data, no account
just check             # formatting, compilation, Clippy
```

See [setup](docs/microsoft-setup.md), [development](docs/development.md), [architecture](docs/architecture.md), and [DESIGN.md](DESIGN.md). Calls, meetings, channels, attachment transfer, and Teams Personal are outside the current scope.
