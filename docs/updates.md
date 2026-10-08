# Automatic macOS updates

## User behavior

Version 0.4.1 is the first updater-enabled release. The 0.4.0 workflow was cancelled before publication when an upstream minimum-OS increase was detected. Install it manually once; 0.3.0 and earlier cannot acquire an updater automatically. The signed release app checks automatically and Sparkle downloads eligible updates. Its native interface provides installation/restart controls. **TeamsFast → Check for Updates…**, the command palette, and **Settings → Advanced → App updates** start a manual check. Automatic checking can be turned off in Settings; Sparkle owns these preferences, rather than duplicating them in our JSON settings.

The update replaces the complete `.app`, not just its executable. Chat data and Keychain entries stay outside the bundle. Before a requested restart, TeamsFast saves a complete snapshot of drafts/unconfirmed sends on the storage worker and waits for acknowledgement. Failures keep the app open; cancelling the save keeps working, and Check for Updates can retry the postponed restart. Finish or dismiss an open dialog first. Normal Quit/window close use the same gate. OS-initiated termination has only GPUI's 200 ms best-effort cleanup window, not an unlimited save guarantee.

## Integration

- `src/updates.rs` retains `sparkle-updater` 0.1.0's safe main-thread controller and relaunch continuation. No Tauri, WebView or custom FFI is introduced.
- Sparkle 2.9.6 supplies its own native UI, downloads, verification and installer. No custom archive extractor, binary replacer or update daemon is added.
- `auto-update` is optional for ordinary development. `cargo run` and `just app` do not need the native framework and cannot self-update. Demo mode also disables updater startup.
- The release script downloads the pinned upstream framework and verifies its SHA-256, signature and minimum OS. Version 2.9.6 supports macOS 10.13+, preserving TeamsFast's macOS 11 minimum; 2.10 requires macOS 12 and must not be substituted without an explicit compatibility change. `build.rs` adds the bundle-relative framework runpath.
- CI checks both default and updater-enabled compilation/Clippy. `scripts/release_macos.sh` enables the feature and copies the full framework, including helpers/XPC services, into `Contents/Frameworks`.

## Signing and feeds

There are two independent layers:

1. Developer ID signing and Apple notarization cover the app and nested code. Helpers are signed inside-out, retaining necessary sandbox entitlements, before the framework and outer bundle. The final archive contains the stapled app.
2. Sparkle Ed25519 signs the final update archive and appcast XML. `SUVerifyUpdateBeforeExtraction` and `SURequireSignedFeed` are enabled. SHA-256 sidecars remain useful for manual downloads, but are not the authenticity mechanism.

The public key is `packaging/macos/sparkle-public-key.txt` and is embedded in `Info.plist`. The private key is in Keychain under account `dev.teamsfast.desktop`, backed up in 1Password as **TeamsFast Sparkle signing**, and supplied to CI as **SPARKLE_PRIVATE_KEY**. It is never written to `.env`, source, build artifacts or logs. The release script passes it to Sparkle tools through stdin. Preserve this key for future updates; do not rotate it casually.

Each published release includes `appcast-arm64.xml` and `appcast-x86_64.xml`. The app embeds its architecture's stable HTTPS URL:

- `https://github.com/ggagosh/teams-fast/releases/latest/download/appcast-arm64.xml`
- `https://github.com/ggagosh/teams-fast/releases/latest/download/appcast-x86_64.xml`

Each signed feed points to the exact versioned GitHub Release archive. Sparkle's `generate_appcast` derives version, minimum OS and hardware requirements from the bundle; the script verifies the signed XML and expected version/asset/size before publication. No GitHub credential is shipped in the app. Release builds fail if the CI signing secret is missing. A local package without it has no generated appcast and is not a publishable update.

## Local checks and acceptance

```sh
just check
export SPARKLE_FRAMEWORK_PATH="$(scripts/setup_sparkle.sh)"
cargo clippy --workspace --all-targets --features auto-update --locked -- -D warnings
cargo test --workspace --locked
```

The initial implementation passed default/updater-enabled checks and all 11 existing tests. A local release package was assembled and its appcast was generated and signature-verified with Sparkle. The feed's version, CPU, archive URL and byte length matched the package; a deliberately modified feed was rejected. No permanent tests were added.

Do not confuse package/signature checks with an installed upgrade. Acceptance still requires installing updater-enabled version N, offering signed N+1, choosing update/restart, then confirming the running version, restored drafts and pending sends. Also check offline/invalid feeds, wrong architecture, invalid signatures, cancellation, denied writes, denied Keychain access, and a send in flight. Use isolated signed bundles and synthetic data, not a second live Graph connection. Notifications, relay protocol, and live Graph behavior are separate checks and were not changed by this integration.

## References

- [GPUI Kit auto-update responsibilities](https://gpui-kit.com/docs/auto-update/)
- [GPUI Kit packaging](https://gpui-kit.com/docs/packaging/)
- [Sparkle setup and signing](https://sparkle-project.org/documentation/)
- [Publishing updates](https://sparkle-project.org/documentation/publishing/)
- [Rust main-thread binding and relaunch lifecycle](https://docs.rs/sparkle-updater/0.1.0/sparkle_updater/)
