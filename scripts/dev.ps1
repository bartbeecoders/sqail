# Test DBs up, service in the background, UI in front. Closing the UI stops the service.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
& "$PSScriptRoot/db.ps1" up
Invoke-Checked cargo build --workspace
Info 'starting sqail-service'
$service = Start-Process (Join-Path $Root 'target/debug/sqail-service.exe') -ArgumentList 'serve' -NoNewWindow -PassThru
# Wait until it answers, so the UI uses it instead of starting its own.
for ($i = 0; $i -lt 50; $i++) {
    curl.exe -skf "$ServiceUrl/v1/health" *> $null
    if ($LASTEXITCODE -eq 0) { break }
    if ($service.HasExited) { Die 'sqail-service exited (is port 7443 in use?)' }
    Start-Sleep -Milliseconds 200
}
try {
    Info 'starting sqail UI'
    $env:SQAIL_AUTO_LOCAL = '1'
    cargo run -q -p sqail-ui --bin sqail
} finally {
    Stop-Process -Id $service.Id -ErrorAction SilentlyContinue
}
