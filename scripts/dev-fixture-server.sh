#!/usr/bin/env bash
# Development-only launcher for the local fixture server.
#
# Writes a plain-text sheet of every client-facing URL, username, password, and
# API key for this fixture to $FIXTURE_ROOT/ENDPOINTS.md (always overwritten),
# then execs the server. Never use this against a real database: it prints
# credentials in clear text by design.
set -euo pipefail

STUMP_ROOT="${STUMP_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
FIXTURE_ROOT="${FIXTURE_ROOT:-$HOME/.local/share/stump-komga-test}"
BIN="${STUMP_BIN:-$STUMP_ROOT/target/debug/stump_server}"
PORT="${STUMP_PORT:-25600}"
LAN_IP="${LAN_IP:-$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1)}"
BASE="http://${LAN_IP:-127.0.0.1}:$PORT"

# Fixture identities. Override via env to match a reseeded database.
OWNER_USER="${OWNER_USER:-synthetic-owner}"
OWNER_PASS="${OWNER_PASS:-synthetic-pass-1}"
SECOND_USER="${SECOND_USER:-k}"
SECOND_PASS="${SECOND_PASS:-k1234567}"
API_KEY="${API_KEY:-stump_VGQQpvVAkun_KyDK1NExypMM2cQJA7XtcXCzHgEiWczYc}"

mkdir -p "$FIXTURE_ROOT/config"
SHEET="$FIXTURE_ROOT/ENDPOINTS.md"

# Owner-only from the first byte: create the temp file under a restrictive
# umask, then replace the sheet atomically so no reader ever sees a partial
# or world-readable file.
ORIGINAL_UMASK="$(umask)"
umask 077
SHEET_TMP="$(mktemp "$FIXTURE_ROOT/.ENDPOINTS.md.XXXXXX")"
trap 'rm -f "$SHEET_TMP"' EXIT

cat >"$SHEET_TMP" <<EOF
# Stump fixture endpoints (generated $(date -u +%FT%TZ) by scripts/dev-fixture-server.sh)

DEVELOPMENT ONLY. Credentials in clear text. Regenerated on every start.

Server base: $BASE
Loopback:    http://127.0.0.1:$PORT
Binary:      $BIN
Fixture:     $FIXTURE_ROOT
Enabled:     STUMP_ENABLE_KOMGA=true STUMP_ENABLE_KAVITA=true ENABLE_KOBO_SYNC=true ENABLE_KOREADER_SYNC=true STUMP_ENABLE_ABS=true KOBO_KEPUB_CONVERSION=true STUMP_ENABLE_UPLOAD=true STUMP_ENABLE_BACKGROUND_JOBS=true STUMP_ENABLE_PROVIDERS=false (+ liseur-sync compiled in)

## Accounts

| Role         | Username        | Password         |
| ------------ | --------------- | ---------------- |
| server owner | $OWNER_USER | $OWNER_PASS |
| user         | $SECOND_USER | $SECOND_PASS |

API key (owner, all scopes): $API_KEY

## Komelia (Komga profile)

Server URL: $BASE
Login:      $OWNER_USER / $OWNER_PASS  (or $SECOND_USER / $SECOND_PASS)
Notes:      log out and back in once after a server rebuild so the app stores the
            komga-remember-me token; force-close before testing background downloads.

## Mihon / Tachiyomi (Keiyoushi "Komga" extension)

Server URL: $BASE
Login:      $OWNER_USER / $OWNER_PASS
Notes:      no trailing path; the extension uses Basic auth on every request.

## Liseur

Komga server:      URL $BASE, API key $API_KEY  (X-API-Key)
Grimmory server:   URL $BASE, OPDS user $OWNER_USER / $OWNER_PASS
                   (client talks to $BASE/komga/api/...)
liseur-sync server: URL $BASE, login $OWNER_USER / $OWNER_PASS
                   (POST /v1/login -> bearer token)

## OPDS (any reader, Basic auth $OWNER_USER / $OWNER_PASS)

OPDS 1.2 catalog:  $BASE/opds/v1.2/catalog
OPDS 2.0 catalog:  $BASE/opds/v2.0/catalog
Key-in-URL (no Basic needed):
                   $BASE/opds/$API_KEY/v1.2/catalog

## KOReader (KOSync plugin)

Sync server: $BASE/koreader/$API_KEY
Username / password: anything (the key in the URL authenticates)

## Kobo (stock firmware, api_endpoint in .kobo/Kobo/Kobo eReader.conf)

api_endpoint=$BASE/kobo/$API_KEY
Expect: books + covers. Progress/shelves are not persisted yet (documented gap).

## Native Stump

Health:   $BASE/api/v2/health
GraphQL:  $BASE/api/graphql  (session or Bearer)
Login:    POST $BASE/api/v2/auth/login  {"username","password"}
EOF
chmod 600 "$SHEET_TMP"
mv -f "$SHEET_TMP" "$SHEET"
trap - EXIT
umask "$ORIGINAL_UMASK"

echo "wrote $SHEET"

export STUMP_CONFIG_DIR="${STUMP_CONFIG_DIR:-$FIXTURE_ROOT/config}"
export STUMP_DB_PATH="${STUMP_DB_PATH:-$FIXTURE_ROOT/config}"
export STUMP_PORT="$PORT"
export STUMP_VERBOSITY="${STUMP_VERBOSITY:-3}"
export STUMP_ENABLE_KOMGA="${STUMP_ENABLE_KOMGA:-true}"
export STUMP_ENABLE_KAVITA="${STUMP_ENABLE_KAVITA:-true}"
export ENABLE_KOBO_SYNC="${ENABLE_KOBO_SYNC:-true}"
export KOBO_KEPUB_CONVERSION="${KOBO_KEPUB_CONVERSION:-true}"
export PDFIUM_PATH="${PDFIUM_PATH:-/tmp/libpdfium.so}"
export STUMP_ENABLE_UPLOAD="${STUMP_ENABLE_UPLOAD:-true}"
export INGEST_EDITOR_DIR="${INGEST_EDITOR_DIR:-$STUMP_ROOT/editor/build}"
export STUMP_HOME_APP_DIR="${STUMP_HOME_APP_DIR:-$STUMP_ROOT/home/build}"
export ENABLE_KOREADER_SYNC="${ENABLE_KOREADER_SYNC:-true}"
export STUMP_ENABLE_ABS="${STUMP_ENABLE_ABS:-true}"
export STUMP_ENABLE_BACKGROUND_JOBS="${STUMP_ENABLE_BACKGROUND_JOBS:-true}"
export STUMP_ENABLE_PROVIDERS="${STUMP_ENABLE_PROVIDERS:-false}"

cd "$FIXTURE_ROOT"
exec "$BIN"
