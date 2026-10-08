# Changelog

## [Unreleased]

## [0.5.3]

### Changed
- Polished the conversation switcher and command palette with a unified surface, compact rows, avatar unread badges and clearer keyboard hints.

## [0.5.2]

### Fixed
- Correctly recognize Graph's masked system-event enum values, so old call-only meetings are no longer mistaken for conversations containing messages. Automatically recheck the incorrect classification cached by 0.5.1 without deleting history or drafts.
- Show labeled activity rows for structured meeting events even when Graph returns `unknownFutureValue`.

### Upgrade notes
- Version 0.5.1 cached an incorrect answer for call-only meetings. This update automatically rechecks that classification; older entries can remain visible until background checks finish. Search by name to reopen filtered meetings.
- No reset, sign-out or data deletion is needed. History, drafts, pending sends and Keychain entries are preserved.

## [0.5.1]

### Fixed
- Old call-only meeting threads stay out of the default sidebar and switcher after 30 days, while remaining searchable by name. Full history checks preserve meetings with real messages, and drafts and pending sends remain visible.

### Upgrade notes
- On the first refresh, old call-only meetings may remain listed until background history checks finish. Search by name in the sidebar or ⌘K to reopen them. The 30-day cutoff is a TeamsFast policy, not an exact replica of Teams' private list rules.
- No reset, sign-out or data deletion is needed; existing history, drafts and pending sends are preserved.

## [0.5.0]

### Fixed
- System events no longer create unread badges, desktop alerts, or empty “Teams” message bubbles.
- Hidden chats stay out of the sidebar and switcher; complete refreshes remove stale cached entries without discarding drafts or unconfirmed sends.
- Notification settings distinguish denied macOS permission from bundle or posting errors, show test-request status, and link to macOS Notification Settings.

### Changed
- Closing the main window hides TeamsFast and keeps updates running; reopen it from the Dock or a notification, and use ⌘Q to quit safely.
- Outgoing messages show **Sent to Teams**, not a recipient delivery/read claim. Typing and recipient receipts remain unavailable through the supported Graph interface.

### Upgrade notes
- The red close button now keeps TeamsFast running; use ⌘Q to quit. Existing drafts, encrypted history and Keychain entries are preserved.
- macOS notification permission is separate from the in-app switch. Enable TeamsFast in System Settings → Notifications if permission was denied.

## [0.4.2]

### Fixed
- Fixed a startup crash when restoring a remembered account: dialog cleanup now waits until the native window's component state exists.

### Upgrade notes
- If 0.4.1 crashes at launch, install 0.4.2 manually. Keep existing settings, history and Keychain entries; no data reset or sign-out is needed.

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
