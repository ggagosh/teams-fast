# Changelog

## [Unreleased]

## [0.4.1]

### Added
- Native Sparkle updates: automatic checks/downloads, signed update feeds, and Check for Updates in the app menu, command palette and Settings.
- Quitting, closing the main window and update restarts wait for encrypted drafts and unconfirmed sends to be saved; failed saves leave the app open.

### Upgrade notes
- Sparkle 2.9.6 preserves macOS 11 support. The 0.4.0 release job was cancelled before publication after detecting that Sparkle 2.10 requires macOS 12.
- Install this version manually once. Earlier versions have no updater; future releases can update the complete signed app in place.
- Update downloads and feeds are signed separately from Apple code signing. Apple Silicon and Intel use separate feeds.

## [0.3.0]

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
