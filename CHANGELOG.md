# Changelog

## [Unreleased]

### Added
- ⌘K conversation switching with recent chats, fuzzy name search, avatars and unread hints.
- ⌘⇧P app commands, ⌘/ shortcut help, and native Go and Help menus.
- Encrypted local history and drafts, offline cached reading, and durable unconfirmed-send recovery without automatic resending.
- Optimistic reactions, plain-text editing and deletion of your own messages, plus best-effort read-state synchronization.
- Clear downloaded history in Settings while preserving drafts and pending sends.
- Profile photos and one Microsoft consent flow for reading, sending and starting chats.

### Changed
- Sidebar filtering uses ⌘⇧F; ⌘F is reserved for future message search.
- Debug and release builds use separate Keychain entries.

### Upgrade notes
- History encryption uses a per-account key in macOS Keychain. Existing plaintext drafts are removed only after encrypted migration succeeds.
- Existing accounts may need to sign in again to grant the expanded chat/profile permissions.
- Cached history is bounded, not a full offline replica. Read synchronization is best-effort; uncertain sends are never retried automatically.
