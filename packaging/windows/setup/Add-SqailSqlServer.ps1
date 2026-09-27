<#
.SYNOPSIS
    Register a Microsoft SQL Server database with sqail-service.

.DESCRIPTION
    Tests the connection from the service host first, then saves it as a
    connection profile that every sqail user of this service can use. The
    password is sent to the service only, which stores it encrypted; it is
    never written to disk here and never shown again.

    Needs an admin token (the one printed by Install-SqailService.ps1, or
    New-SqailToken.ps1 -Scope admin). You are prompted for it and for the
    password when they are not given. Run it on the service host; for a
    remote service pass -CaCert (the service's certificate, e.g. a copy of
    <DataDir>\tls\dev-cert.pem).

    Works in Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
    .\Add-SqailSqlServer.ps1 -Name "Sales (prod)" -Server SQL01 -Database Sales -User sqail_reader -ReadOnly -Environment prod -Color "#c0392b"
.EXAMPLE
    .\Add-SqailSqlServer.ps1 -Name "HR" -Server 'SQL02\HR' -Database HR -WindowsAuth
    Named instance (found via SQL Browser), Windows authentication as the
    service account.
.EXAMPLE
    .\Add-SqailSqlServer.ps1 -Name "Dev" -Server 'devbox,14330' -Database App -User sa -TrustServerCertificate
#>
[CmdletBinding()]
param(
    # Name shown in sqail.
    [Parameter(Mandatory = $true)] [string] $Name,
    # Host name or IP. Also accepts HOST\INSTANCE and HOST,PORT (as in SSMS).
    [Parameter(Mandatory = $true)] [string] $Server,
    [string] $Database,
    # Named instance (alternative to HOST\INSTANCE). Needs SQL Browser (UDP 1434).
    [string] $Instance,
    [int] $Port = 1433,
    # SQL Server login. Omit together with -WindowsAuth.
    [string] $User,
    [securestring] $Password,
    # Windows integrated authentication, as the account the service runs as.
    [switch] $WindowsAuth,
    # Required (default): always TLS. On: TLS if the server offers it. Off: login only.
    [ValidateSet('Required', 'On', 'Off')] [string] $Encrypt = 'Required',
    # Accept the SQL Server's certificate without validation (test servers only).
    [switch] $TrustServerCertificate,
    [switch] $ReadOnly,
    [string] $Environment,
    # Accent colour in sqail, e.g. "#c0392b" for production.
    [string] $Color,
    [string] $Folder,
    [string] $ServiceUrl = 'https://127.0.0.1:7443',
    # Admin token; default: $env:SQAIL_TOKEN, else a prompt.
    [string] $Token,
    # PEM certificate to verify a remote service with.
    [string] $CaCert,
    # Save the profile even if the connection test fails.
    [switch] $Force
)

$ErrorActionPreference = 'Stop'
function Fail($m) { Write-Host "err $m" -ForegroundColor Red; exit 1 }
function Plain([securestring] $s) {
    $b = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($s)
    try { [Runtime.InteropServices.Marshal]::PtrToStringBSTR($b) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($b) }
}

if (-not $WindowsAuth -and -not $User) { Fail 'Give -User (SQL Server login) or -WindowsAuth.' }
if ($WindowsAuth -and $User) { Fail '-User and -WindowsAuth are exclusive.' }

$ServiceUrl = $ServiceUrl.TrimEnd('/')
$uri = [Uri]$ServiceUrl
$loopback = @('127.0.0.1', 'localhost', '::1', '[::1]') -contains $uri.Host
if (-not $CaCert -and -not $loopback) {
    Fail "For a remote service pass -CaCert <service certificate PEM>; without it the connection could not be verified."
}

if (-not $Token) { $Token = $env:SQAIL_TOKEN }
if (-not $Token) { $Token = Plain (Read-Host 'Admin token (sq2_...)' -AsSecureString) }
$pw = $null
if (-not $WindowsAuth) {
    if (-not $Password) { $Password = Read-Host "Password for SQL login '$User'" -AsSecureString }
    $pw = Plain $Password
}

$auth = if ($WindowsAuth) { @{ method = 'integrated' } } else { @{ method = 'sql'; user = $User } }
$params = [ordered]@{
    engine                   = 'mssql'
    host                     = $Server
    port                     = $Port
    auth                     = $auth
    encrypt                  = $Encrypt.ToLowerInvariant()
    trust_server_certificate = [bool]$TrustServerCertificate
}
if ($Instance) { $params.instance = $Instance }
if ($Database) { $params.database = $Database }
$body = [ordered]@{ name = $Name; params = $params; read_only = [bool]$ReadOnly }
if ($null -ne $pw) { $body.password = $pw }
if ($Environment) { $body.environment = $Environment }
if ($Color) { $body.color = $Color }
if ($Folder) { $body.folder = $Folder }
$json = $body | ConvertTo-Json -Depth 5 -Compress

# curl.exe ships with Windows 10+. Token and password go in a config read
# from stdin, so they never appear on a command line.
$curl = if ($env:OS -eq 'Windows_NT') { 'curl.exe' } else { 'curl' }
function Invoke-Api([string] $Method, [string] $Path, [string] $Json) {
    function q([string] $v) { '"' + $v.Replace('\', '\\').Replace('"', '\"') + '"' }
    $cfg = @(
        "url = $(q ($ServiceUrl + $Path))",
        "request = $(q $Method)",
        "header = $(q ('Authorization: Bearer ' + $Token))",
        'header = "content-type: application/json"'
    )
    if ($Json) { $cfg += "data-binary = $(q $Json)" }
    # (An if-expression would unroll a one-element array into a string.)
    $tls = @('-k')
    if ($CaCert) { $tls = @('--cacert', $CaCert) }
    $oldIn = $OutputEncoding; $oldOut = [Console]::OutputEncoding
    $utf8 = New-Object Text.UTF8Encoding $false
    try {
        $OutputEncoding = $utf8; [Console]::OutputEncoding = $utf8
        $out = ($cfg -join "`n") | & $curl -sS --max-time 60 -w "`n%{http_code}" -K - @tls 2>&1
    } finally {
        $OutputEncoding = $oldIn; [Console]::OutputEncoding = $oldOut
    }
    if ($LASTEXITCODE -ne 0) { Fail "cannot reach $ServiceUrl ($out)" }
    $lines = @($out | ForEach-Object { "$_" })
    $status = [int]$lines[-1]
    $text = ''
    if ($lines.Count -gt 1) { $text = ($lines[0..($lines.Count - 2)] -join "`n") }
    $data = $null
    if ($text.Trim()) { $data = $text | ConvertFrom-Json }
    if ($status -ge 400) {
        $detail = if ($data -and $data.detail) { $data.detail } else { $text }
        if ($status -eq 401) { $detail = 'the token is not valid for this service' }
        Fail "$Method $Path -> $status $detail"
    }
    return $data
}

Write-Host "==> testing $Server from the service host" -ForegroundColor Cyan
$test = Invoke-Api 'POST' '/v1/connections/test' $json
if ($test.ok) {
    Write-Host " ok connected in $($test.latency_ms) ms: $($test.server_version)" -ForegroundColor Green
} else {
    Write-Host "err $($test.error)" -ForegroundColor Red
    $e = "$($test.error)"
    if ($e -match '18456|Login failed') {
        Write-Host '    Hint: wrong login/password, SQL authentication disabled on the server (mixed mode), or no access to the database.'
    } elseif ($e -match 'certificate|TLS|tls|handshake') {
        Write-Host '    Hint: the server certificate is not trusted by the service host. Install a trusted certificate on SQL Server, or use -TrustServerCertificate for test servers.'
    } elseif ($e -match 'browser|1434') {
        Write-Host '    Hint: SQL Browser did not answer. Start the "SQL Server Browser" service and allow UDP 1434, or give the port: -Server "HOST,PORT".'
    } elseif ($e -match 'timed out|refused|No such host|resolve') {
        Write-Host '    Hint: check the host name, that TCP/IP is enabled in SQL Server Configuration Manager, the port, and the firewall between this host and SQL Server.'
    }
    if (-not $Force) { Fail 'not saved (use -Force to save anyway)' }
}

$conn = Invoke-Api 'POST' '/v1/connections' $json
Write-Host " ok saved '$($conn.name)' (id $($conn.id))" -ForegroundColor Green
Write-Host "    Users of $ServiceUrl now see it under Connections in sqail (Connections > Refresh)."
