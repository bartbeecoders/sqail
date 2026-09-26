#!/usr/bin/env bash
# Manage the podman test databases.
#   scripts/db.sh up       start + seed Postgres, SQL Server and the SQLite file
#   scripts/db.sh down     stop containers (data kept)
#   scripts/db.sh reset    destroy containers + data, then `up` again
#   scripts/db.sh destroy  remove containers, volumes and the sqlite file
#   scripts/db.sh status   show containers and connection strings
#   scripts/db.sh logs     follow container logs
#   scripts/db.sh psql     open psql on the test Postgres
#   scripts/db.sh sqlcmd   open sqlcmd on the test SQL Server
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

wait_for() {  # name, command...
    local name=$1; shift
    printf '    waiting for %s ' "$name"
    for _ in $(seq 1 90); do
        if "$@" >/dev/null 2>&1; then echo; ok "$name ready"; return 0; fi
        printf '.'; sleep 2
    done
    echo; die "$name did not become ready (scripts/db.sh logs)"
}

sqlcmd_in() { podman exec sqail2-mssql /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -b "$@"; }

seed_mssql() {
    if [[ "$(sqlcmd_in -h -1 -W -Q "SET NOCOUNT ON; SELECT CASE WHEN OBJECT_ID('sqail_test.sales.type_zoo') IS NULL THEN 0 ELSE 1 END")" == "1" ]]; then
        ok "mssql already seeded"; return
    fi
    info "seeding SQL Server"
    for f in "$ROOT"/dev/seed/mssql/*.sql; do
        sqlcmd_in -i "/seed/$(basename "$f")" >/dev/null \
            || die "seeding $(basename "$f") failed — fix it, then: scripts/db.sh reset"
    done
    ok "mssql seeded"
}

seed_sqlite() {
    local db="$DATA_DIR/sqail_test.db"
    mkdir -p "$DATA_DIR"
    if [[ -f "$db" ]]; then ok "sqlite already seeded ($db)"; return; fi
    info "building SQLite test database"
    if command -v sqlite3 >/dev/null; then
        sqlite3 "$db" < "$ROOT/dev/seed/sqlite/seed.sql"
    else  # fall back to a throwaway container
        podman run --rm -v "$DATA_DIR:/data" -v "$ROOT/dev/seed/sqlite:/seed:ro" docker.io/library/alpine:3 \
            sh -c 'apk add -q sqlite && sqlite3 /data/sqail_test.db < /seed/seed.sql'
    fi
    ok "sqlite seeded ($db)"
}

status() {
    podman ps -a --filter name=sqail2- --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
    cat <<INFO

  postgres  postgres://sqail:sqail_dev_pw@127.0.0.1:55432/sqail_test
  mssql     server=127.0.0.1,51433  db=sqail_test  user=sqail|sa  pw=$MSSQL_SA_PASSWORD
  sqlite    $DATA_DIR/sqail_test.db
INFO
}

# Containers are defined here with plain `podman run` — no compose provider or
# podman socket needed, and the same definitions work on Windows (db.ps1).
# DEV ONLY: these credentials are throwaway. Ports bind to 127.0.0.1 only and
# are non-standard so they never clash with a real local server.
start_postgres() {
    if podman container exists sqail2-postgres; then podman start sqail2-postgres >/dev/null; return; fi
    podman run -d --name sqail2-postgres \
        -e POSTGRES_USER=sqail -e POSTGRES_PASSWORD=sqail_dev_pw -e POSTGRES_DB=sqail_test \
        -p 127.0.0.1:55432:5432 \
        -v sqail2-pgdata:/var/lib/postgresql/data \
        -v "$ROOT/dev/seed/postgres:/docker-entrypoint-initdb.d:ro" \
        docker.io/library/postgres:17-alpine >/dev/null
}

start_mssql() {
    if podman container exists sqail2-mssql; then podman start sqail2-mssql >/dev/null; return; fi
    podman run -d --name sqail2-mssql \
        -e ACCEPT_EULA=Y -e MSSQL_PID=Developer -e "MSSQL_SA_PASSWORD=$MSSQL_SA_PASSWORD" \
        -p 127.0.0.1:51433:1433 \
        -v sqail2-mssqldata:/var/opt/mssql \
        -v "$ROOT/dev/seed/mssql:/seed:ro" \
        mcr.microsoft.com/mssql/server:2022-latest >/dev/null
}

down() { podman stop -i -t 30 sqail2-postgres sqail2-mssql >/dev/null; ok "stopped"; }

destroy() {
    podman rm -f -i sqail2-postgres sqail2-mssql >/dev/null
    podman volume rm -f sqail2-pgdata sqail2-mssqldata >/dev/null 2>&1 || true
    rm -rf "$DATA_DIR"
    ok "containers, volumes and sqlite file removed"
}

up() {
    need podman "install podman (omarchy: sudo pacman -S podman)"
    info "starting test databases"
    start_postgres
    start_mssql
    wait_for postgres podman exec sqail2-postgres psql -U sqail -d sqail_test -tAc "SELECT 1 FROM sales.type_zoo LIMIT 1"
    wait_for mssql sqlcmd_in -Q "SELECT 1"
    seed_mssql
    seed_sqlite
    status
}

case "${1:-status}" in
    up)     up ;;
    down)   down ;;
    reset)  destroy; up ;;
    destroy) destroy ;;
    status) status ;;
    logs)   podman logs -f --names sqail2-postgres sqail2-mssql ;;
    psql)   podman exec -it sqail2-postgres psql -U sqail -d sqail_test ;;
    sqlcmd) podman exec -it sqail2-mssql /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P "$MSSQL_SA_PASSWORD" -d sqail_test ;;
    *)      sed -n '2,11p' "$0"; exit 1 ;;
esac
