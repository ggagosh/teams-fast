# Project guidance

- Keep the desktop in one Cargo package and add modules for real responsibilities. The separately deployed relay is the only additional workspace package; keep its server dependencies out of desktop builds.
- Use GPUI Kit components and standard library primitives before adding custom controls or dependencies. GPUI Fast is the selected runtime.
- Microsoft HTTP, authentication, and token handling belong in `src/teams.rs`; relay transport belongs in `src/realtime.rs`. Never block the UI thread.
- Never log tokens, device secrets, or real tenant message bodies. Do not persist credentials in plain files.
- Preserve drafts on failed sends; never retry message POSTs automatically.
- macOS only. Store secrets only in the OS credential store; debug builds are signed with the local Apple Development identity (`scripts/sign_dev.sh`) so Keychain access survives rebuilds. The desktop holds no relay secret: it registers with the user's Microsoft ID token. `.env` holds only public IDs and the relay URL; the relay operator key belongs in server runtime configuration.
- The repository is public. Never commit secrets, signing material, tenant data, or `.env`; CI secrets live in GitHub Actions secrets (sourced from 1Password).
- Reuse `src/ui.rs` components and follow `DESIGN.md`. Keep state/interaction in `src/app.rs` and shared chat behavior in `src/model.rs`.
- Update the relevant `docs/` page when behavior or scope changes.
- Run `just check` (workspace formatting, compilation, and Clippy). Do not write new tests; the user will check the runnable native app. Use the macOS `.app` bundle for notification checks.
- Report local checks, native UI checks, relay protocol checks, and live Graph/OS acceptance separately. Deployment alone does not verify incoming push or notification clicks.
- Use `ast-grep --lang rust -p '<pattern>'` for syntax/structure searches. Use `rg` for plain-text matches. For complex ast-grep rules, consult https://ast-grep.github.io/llms-full.txt.
- Do not commit or push unless requested.
