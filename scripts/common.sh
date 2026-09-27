# Shared helpers for sqail2 scripts. Source, don't execute.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="$ROOT/dev/data"
MSSQL_SA_PASSWORD='Sqail2_dev!Passw0rd'

# Dev defaults for sqail-service: state lives in the repo (git-ignored) and the
# SQLite test database directory is allowed. Override by exporting them first.
export SQAIL_DATA_DIR="${SQAIL_DATA_DIR:-$ROOT/.sqail2/service}"
export SQAIL_SQLITE_DIRS="${SQAIL_SQLITE_DIRS:-$DATA_DIR}"
# sqail2 UI settings/tokens for development, kept apart from a real install.
export SQAIL2_CONFIG_DIR="${SQAIL2_CONFIG_DIR:-$ROOT/.sqail2/ui}"
SERVICE_URL="https://127.0.0.1:7443"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m ok\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31merr\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "'$1' not found — $2"; }
