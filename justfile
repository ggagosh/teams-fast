set dotenv-load := true
set positional-arguments := true

dev *args:
    cargo run --bin teamsfast -- "$@"

check:
    cargo fmt --all --check
    cargo check --workspace --all-targets --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Signed development .app: needed for OS notifications and the Dock badge.
app *args:
    cargo build --bin teamsfast
    mkdir -p target/TeamsFast.app/Contents/MacOS
    cp target/debug/teamsfast target/TeamsFast.app/Contents/MacOS/TeamsFast
    cp packaging/macos/Info.plist target/TeamsFast.app/Contents/Info.plist
    scripts/sign_dev.sh target/TeamsFast.app
    target/TeamsFast.app/Contents/MacOS/TeamsFast "$@"

# Release bundle in dist/; signs and notarizes when the MACOS_*/APPLE_* variables are set.
release target="":
    scripts/release_macos.sh {{target}}
