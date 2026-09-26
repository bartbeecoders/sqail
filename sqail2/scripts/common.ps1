# Shared helpers for sqail2 PowerShell scripts. Dot-source, don't execute.
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$DataDir = Join-Path $Root 'dev/data'
$MssqlSaPassword = 'Sqail2_dev!Passw0rd'

# Dev defaults for sqail-service: state lives in the repo (git-ignored) and the
# SQLite test database directory is allowed. Override by setting them first.
if (-not $env:SQAIL_DATA_DIR) { $env:SQAIL_DATA_DIR = Join-Path $Root '.sqail2/service' }
if (-not $env:SQAIL_SQLITE_DIRS) { $env:SQAIL_SQLITE_DIRS = $DataDir }
if (-not $env:SQAIL2_CONFIG_DIR) { $env:SQAIL2_CONFIG_DIR = Join-Path $Root '.sqail2/ui' }
$ServiceUrl = 'https://127.0.0.1:7443'

function Info($m) { Write-Host "==> $m" -ForegroundColor Blue }
function Ok($m)   { Write-Host " ok $m" -ForegroundColor Green }
function Die($m)  { Write-Host "err $m" -ForegroundColor Red; exit 1 }
function Need($cmd, $hint) { if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { Die "'$cmd' not found - $hint" } }
function Invoke-Checked { & $args[0] @($args | Select-Object -Skip 1); if ($LASTEXITCODE -ne 0) { Die "$($args[0]) failed ($LASTEXITCODE)" } }
