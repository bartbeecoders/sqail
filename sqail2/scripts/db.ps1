# Manage the podman test databases (Windows). Same commands as scripts/db.sh:
#   .\scripts\db.ps1 up | down | reset | destroy | status | logs | psql | sqlcmd
# Requires Podman Desktop / `podman machine` running.
param([string]$Command = 'status')
. "$PSScriptRoot/common.ps1"

function SqlcmdIn { podman exec sqail2-mssql /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P $MssqlSaPassword -b @args }

function WaitFor($name, [scriptblock]$probe) {
    Write-Host -NoNewline "    waiting for $name "
    for ($i = 0; $i -lt 90; $i++) {
        & $probe *> $null
        if ($LASTEXITCODE -eq 0) { Write-Host ''; Ok "$name ready"; return }
        Write-Host -NoNewline '.'; Start-Sleep 2
    }
    Die "$name did not become ready (.\scripts\db.ps1 logs)"
}

function Start-Postgres {
    podman container exists sqail2-postgres
    if ($LASTEXITCODE -eq 0) { podman start sqail2-postgres | Out-Null; return }
    Invoke-Checked podman run -d --name sqail2-postgres `
        -e POSTGRES_USER=sqail -e POSTGRES_PASSWORD=sqail_dev_pw -e POSTGRES_DB=sqail_test `
        -p 127.0.0.1:55432:5432 -v sqail2-pgdata:/var/lib/postgresql/data `
        -v "$Root/dev/seed/postgres:/docker-entrypoint-initdb.d:ro" `
        docker.io/library/postgres:17-alpine
}

function Start-Mssql {
    podman container exists sqail2-mssql
    if ($LASTEXITCODE -eq 0) { podman start sqail2-mssql | Out-Null; return }
    Invoke-Checked podman run -d --name sqail2-mssql `
        -e ACCEPT_EULA=Y -e MSSQL_PID=Developer -e "MSSQL_SA_PASSWORD=$MssqlSaPassword" `
        -p 127.0.0.1:51433:1433 -v sqail2-mssqldata:/var/opt/mssql `
        -v "$Root/dev/seed/mssql:/seed:ro" `
        mcr.microsoft.com/mssql/server:2022-latest
}

function Seed-Mssql {
    $seeded = SqlcmdIn -h -1 -W -Q "SET NOCOUNT ON; SELECT CASE WHEN OBJECT_ID('sqail_test.sales.type_zoo') IS NULL THEN 0 ELSE 1 END"
    if ("$seeded".Trim() -eq '1') { Ok 'mssql already seeded'; return }
    Info 'seeding SQL Server'
    foreach ($f in Get-ChildItem "$Root/dev/seed/mssql/*.sql" | Sort-Object Name) {
        SqlcmdIn -i "/seed/$($f.Name)" | Out-Null
        if ($LASTEXITCODE -ne 0) { Die "seeding $($f.Name) failed - fix it, then: .\scripts\db.ps1 reset" }
    }
    Ok 'mssql seeded'
}

function Seed-Sqlite {
    $db = Join-Path $DataDir 'sqail_test.db'
    New-Item -ItemType Directory -Force $DataDir | Out-Null
    if (Test-Path $db) { Ok "sqlite already seeded ($db)"; return }
    Info 'building SQLite test database'
    if (Get-Command sqlite3 -ErrorAction SilentlyContinue) {
        Get-Content -Raw "$Root/dev/seed/sqlite/seed.sql" | sqlite3 $db
    } else {
        Invoke-Checked podman run --rm -v "${DataDir}:/data" -v "$Root/dev/seed/sqlite:/seed:ro" docker.io/library/alpine:3 `
            sh -c 'apk add -q sqlite && sqlite3 /data/sqail_test.db < /seed/seed.sql'
    }
    Ok "sqlite seeded ($db)"
}

function Show-Status {
    podman ps -a --filter name=sqail2- --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
    Write-Host ''
    Write-Host '  postgres  postgres://sqail:sqail_dev_pw@127.0.0.1:55432/sqail_test'
    Write-Host "  mssql     server=127.0.0.1,51433  db=sqail_test  user=sqail|sa  pw=$MssqlSaPassword"
    Write-Host "  sqlite    $DataDir\sqail_test.db"
}

function Destroy {
    podman rm -f -i sqail2-postgres sqail2-mssql | Out-Null
    podman volume rm -f sqail2-pgdata sqail2-mssqldata 2>$null | Out-Null
    Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue
    Ok 'containers, volumes and sqlite file removed'
}

function Up {
    Need podman 'install Podman Desktop and run: podman machine init; podman machine start'
    Info 'starting test databases'
    Start-Postgres
    Start-Mssql
    WaitFor postgres { podman exec sqail2-postgres psql -U sqail -d sqail_test -tAc 'SELECT 1 FROM sales.type_zoo LIMIT 1' }
    WaitFor mssql { SqlcmdIn -Q 'SELECT 1' }
    Seed-Mssql
    Seed-Sqlite
    Show-Status
}

switch ($Command) {
    'up'      { Up }
    'down'    { podman stop -i -t 30 sqail2-postgres sqail2-mssql | Out-Null; Ok 'stopped' }
    'reset'   { Destroy; Up }
    'destroy' { Destroy }
    'status'  { Show-Status }
    'logs'    { podman logs -f --names sqail2-postgres sqail2-mssql }
    'psql'    { podman exec -it sqail2-postgres psql -U sqail -d sqail_test }
    'sqlcmd'  { podman exec -it sqail2-mssql /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P $MssqlSaPassword -d sqail_test }
    default   { Get-Content $PSCommandPath -TotalCount 3; exit 1 }
}
