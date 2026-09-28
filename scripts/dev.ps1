# Test DBs up, service in the background, UI in front. Closing the UI stops the service.
# Opens the admin page signed in. The token is reused from the service data dir
# (.sqail/service/dev-admin-token.txt); a rejected one is replaced.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
$DevAdminTokenFile = Join-Path $env:SQAIL_DATA_DIR 'dev-admin-token.txt'

function Read-DevAdminToken {
    if (-not (Test-Path $DevAdminTokenFile)) { return $null }
    $token = ([System.IO.File]::ReadAllText($DevAdminTokenFile)).Trim()
    if ($token.StartsWith('sq2_')) { return $token }
    return $null
}

function New-DevAdminToken {
    $bin = Join-Path $Root 'target/debug/sqail-service.exe'
    $token = (& $bin token create --name dev --scope admin | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $token.StartsWith('sq2_')) { Die 'could not create a dev admin token' }
    $dir = Split-Path $DevAdminTokenFile
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($DevAdminTokenFile, $token, $utf8)
    return $token
}

function Test-AdminToken([string]$Token) {
    curl.exe -skf -o NUL -H "Authorization: Bearer $Token" "$ServiceUrl/v1/admin/status" *> $null
    $LASTEXITCODE -eq 0
}

& "$PSScriptRoot/db.ps1" up
Invoke-Checked cargo build --workspace
# Before the first start, so the service does not also print a one-time bootstrap token.
$token = Read-DevAdminToken
if (-not $token) { $token = New-DevAdminToken }
Info 'starting sqail-service'
$service = Start-Process (Join-Path $Root 'target/debug/sqail-service.exe') -ArgumentList 'serve' -NoNewWindow -PassThru
# Wait until it answers, so the UI uses it instead of starting its own.
$up = $false
for ($i = 0; $i -lt 50; $i++) {
    curl.exe -skf "$ServiceUrl/v1/health" *> $null
    if ($LASTEXITCODE -eq 0) { $up = $true; break }
    if ($service.HasExited) { Die 'sqail-service exited (is port 7443 in use?)' }
    Start-Sleep -Milliseconds 200
}
try {
    if ($up) {
        if (-not (Test-AdminToken $token)) {
            Info 'saved admin token was rejected; creating a new one'
            $token = New-DevAdminToken
        }
        Info 'opening the admin page'
        Start-Process "$ServiceUrl/admin/#token=$token"
    }
    Info 'starting sqail UI'
    $env:SQAIL_AUTO_LOCAL = '1'
    cargo run -q -p sqail-ui --bin sqail
} finally {
    Stop-Process -Id $service.Id -ErrorAction SilentlyContinue
}
