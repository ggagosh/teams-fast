#!/usr/bin/env bash
# Signs a debug binary or .app with the local Apple Development identity and a designated
# requirement on this app's identifier and team. Keychain access follows that requirement, so
# rebuilt binaries keep access to saved sign-ins without password prompts.
# Override the identity with TEAMSFAST_DEV_SIGNING_IDENTITY (name or SHA-1).
set -euo pipefail

target="$1"
identities="$(/usr/bin/security find-identity -v -p codesigning)"
identity_record="$(printf '%s\n' "$identities" | awk \
    -v requested="${TEAMSFAST_DEV_SIGNING_IDENTITY:-}" '
    /^[[:space:]]*[0-9]+\)/ {
        fingerprint = $2
        name = $0
        sub(/^[^"]*"/, "", name)
        sub(/".*$/, "", name)
        if ((requested == "" && name ~ /^Apple Development:/) ||
            (requested != "" && (name == requested || toupper(fingerprint) == toupper(requested)))) {
            print fingerprint
            print name
            exit
        }
    }')"
if [[ -z "$identity_record" ]]; then
    echo "error: no Apple Development signing identity; install one or set TEAMSFAST_DEV_SIGNING_IDENTITY." >&2
    exit 1
fi
identity="${identity_record%%$'\n'*}"
identity_name="${identity_record#*$'\n'}"
team_id="$(/usr/bin/security find-certificate -c "$identity_name" -p \
    | /usr/bin/openssl x509 -noout -subject -nameopt multiline \
    | sed -n 's/^[[:space:]]*organizationalUnitName[[:space:]]*=[[:space:]]*//p' \
    | tr -d '[:space:]')"
if [[ ! "$team_id" =~ ^[A-Z0-9]{10}$ ]]; then
    echo "error: could not determine the Apple team of the signing identity." >&2
    exit 1
fi

requirement="identifier \"dev.teamsfast.desktop.dev\" and anchor apple generic and certificate leaf[subject.OU] = \"$team_id\""
# Canonical form, so the comparison with codesign's output is exact.
requirement="$(/usr/bin/csreq -r "=$requirement" -t)"
current="$(/usr/bin/codesign -d -r- "$target" 2>/dev/null || true)"
# Cargo leaves an unchanged binary in place; don't touch the signing key unless needed.
if [[ "$current" != "designated => $requirement" ]] || \
    ! /usr/bin/codesign --verify --strict -R "=$requirement" "$target" 2>/dev/null; then
    /usr/bin/codesign --force --sign "$identity" \
        --identifier "dev.teamsfast.desktop.dev" \
        --requirements "=designated => $requirement" \
        --timestamp=none "$target"
fi
