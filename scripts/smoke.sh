#!/usr/bin/env bash
# End-to-end smoke test with nothing but curl: start sqail-service on a fresh
# data dir, create a profile per test database, run a query on each, stop.
# Needs the test databases: scripts/db.sh up
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
need curl "install curl"

cargo build -q -p sqail-service
BIN="$ROOT/target/debug/sqail-service"
SMOKE_DIR="$(mktemp -d)"
export SQAIL_DATA_DIR="$SMOKE_DIR" SQAIL_BIND="127.0.0.1:7444" SQAIL_LOG=warn
URL="https://127.0.0.1:7444"
trap '[[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null; rm -rf "$SMOKE_DIR"' EXIT

info "starting sqail-service on $URL"
"$BIN" serve --no-bootstrap-token 2>"$SMOKE_DIR/service.log" &
PID=$!
for _ in $(seq 1 50); do
    curl -skf "$URL/v1/health" >/dev/null 2>&1 && break
    sleep 0.2
done
curl -skf "$URL/v1/health" >/dev/null || { cat "$SMOKE_DIR/service.log"; die "service did not start"; }
ok "health"

TOKEN="$("$BIN" token create --name smoke --scope admin)"
api() { curl -sk -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' "$@"; }
[[ "$(api -o /dev/null -w '%{http_code}' "$URL/v1/info")" == 200 ]] || die "token rejected"
[[ "$(curl -sk -o /dev/null -w '%{http_code}' "$URL/v1/info")" == 401 ]] || die "unauthenticated request accepted"
ok "auth"

create() {  # name, params-json, password → id
    api "$URL/v1/connections" -d "{\"name\":\"$1\",\"params\":$2,\"password\":\"$3\"}" \
        | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4
}
PG=$(create postgres '{"engine":"postgres","host":"127.0.0.1","port":55432,"database":"sqail_test","user":"sqail","ssl_mode":"disable"}' sqail_dev_pw)
MS=$(create mssql '{"engine":"mssql","host":"127.0.0.1","port":51433,"database":"sqail_test","auth":{"method":"sql","user":"sqail"},"trust_server_certificate":true}' "$MSSQL_SA_PASSWORD")
SL=$(create sqlite "{\"engine\":\"sqlite\",\"path\":\"$DATA_DIR/sqail_test.db\"}" "")
[[ -n "$PG" && -n "$MS" && -n "$SL" ]] || die "creating connection profiles failed"
ok "profiles created"

failed=0
for pair in "postgres:$PG:sales.order_items" "mssql:$MS:sales.order_items" "sqlite:$SL:order_items"; do
    IFS=: read -r name id table <<<"$pair"
    out="$(api "$URL/v1/connections/$id/query" -d "{\"sql\":\"SELECT count(*) AS n FROM $table\"}")"
    if grep -q '"rows":\[\[29850\]\]' <<<"$out" && grep -q '"event":"done"' <<<"$out"; then
        ok "$name: SELECT count(*) → 29850"
    else
        printf '%s\n' "$out"; failed=1; printf '\033[1;31merr\033[0m %s query failed\n' "$name"
    fi
done
[[ $failed == 0 ]] || die "smoke test failed"
ok "smoke test passed"
