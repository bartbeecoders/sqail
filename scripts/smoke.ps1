# End-to-end smoke test with nothing but curl.exe: start sqail-service on a
# fresh data dir, create a profile per test database, run a query on each, stop.
# Needs the test databases: .\scripts\db.ps1 up
. "$PSScriptRoot/common.ps1"
Set-Location $Root
Need curl.exe 'curl.exe ships with Windows 10+'

Invoke-Checked cargo build -q -p sqail-service
$bin = Join-Path $Root 'target/debug/sqail-service.exe'
$smokeDir = Join-Path ([IO.Path]::GetTempPath()) ("sqail-smoke-" + [guid]::NewGuid())
New-Item -ItemType Directory $smokeDir | Out-Null
$env:SQAIL_DATA_DIR = $smokeDir; $env:SQAIL_BIND = '127.0.0.1:7444'; $env:SQAIL_LOG = 'warn'
$url = 'https://127.0.0.1:7444'

Info "starting sqail-service on $url"
$svc = Start-Process $bin -ArgumentList 'serve' -PassThru -NoNewWindow -RedirectStandardError (Join-Path $smokeDir 'service.log')
try {
    for ($i = 0; $i -lt 50; $i++) {
        curl.exe -skf "$url/v1/health" *> $null
        if ($LASTEXITCODE -eq 0) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($LASTEXITCODE -ne 0) { Die 'service did not start' }
    Ok 'health'

    $token = (& $bin token create --name smoke --scope admin).Trim()
    function Api($path, $body) {
        $tmp = New-TemporaryFile
        Set-Content -NoNewline -Path $tmp -Value $body
        $out = curl.exe -sk -H "Authorization: Bearer $token" -H 'content-type: application/json' "$url$path" --data-binary "@$tmp"
        Remove-Item $tmp
        return ($out -join "`n")
    }
    function Create($name, $params, $password) {
        $out = Api '/v1/connections' "{`"name`":`"$name`",`"params`":$params,`"password`":`"$password`"}"
        return ($out | ConvertFrom-Json).id
    }
    $sqlitePath = (Join-Path $DataDir 'sqail_test.db') -replace '\\', '/'
    $ids = [ordered]@{
        postgres = Create 'postgres' '{"engine":"postgres","host":"127.0.0.1","port":55432,"database":"sqail_test","user":"sqail","ssl_mode":"disable"}' 'sqail_dev_pw'
        mssql    = Create 'mssql' '{"engine":"mssql","host":"127.0.0.1","port":51433,"database":"sqail_test","auth":{"method":"sql","user":"sqail"},"trust_server_certificate":true}' $MssqlSaPassword
        sqlite   = Create 'sqlite' "{`"engine`":`"sqlite`",`"path`":`"$sqlitePath`"}" ''
    }
    $tables = @{ postgres = 'sales.order_items'; mssql = 'sales.order_items'; sqlite = 'order_items' }
    $failed = $false
    foreach ($name in $ids.Keys) {
        $out = Api "/v1/connections/$($ids[$name])/query" "{`"sql`":`"SELECT count(*) AS n FROM $($tables[$name])`"}"
        if ($out -match '"rows":\[\[29850\]\]' -and $out -match '"event":"done"') { Ok "${name}: SELECT count(*) -> 29850" }
        else { Write-Host $out; Write-Host "err $name query failed" -ForegroundColor Red; $failed = $true }
    }
    if ($failed) { Die 'smoke test failed' }
    Ok 'smoke test passed'
} finally {
    Stop-Process -Id $svc.Id -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $smokeDir -ErrorAction SilentlyContinue
}
