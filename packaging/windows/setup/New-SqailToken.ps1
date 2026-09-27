<#
.SYNOPSIS
    Create an API token for a user or machine (run as Administrator on the
    service host).

.DESCRIPTION
    Scopes:
      read   browse schemas; run SQL only on read-only connection profiles
      query  run SQL, explain and use transactions on every profile (default)
      admin  also manage connection profiles and tokens, read the audit log

    The token is shown once. Hand it over through a secure channel together
    with the service URL and certificate fingerprint printed below.

.EXAMPLE
    .\New-SqailToken.ps1 -Name alice
.EXAMPLE
    .\New-SqailToken.ps1 -Name "reporting server" -Scope read
.EXAMPLE
    .\New-SqailToken.ps1 -List
#>
[CmdletBinding(DefaultParameterSetName = 'Create')]
param(
    [Parameter(ParameterSetName = 'Create', Mandatory = $true, Position = 0)]
    [string] $Name,
    [Parameter(ParameterSetName = 'Create')]
    [ValidateSet('read', 'query', 'admin')]
    [string] $Scope = 'query',
    [Parameter(ParameterSetName = 'List', Mandatory = $true)]
    [switch] $List,
    [Parameter(ParameterSetName = 'Revoke', Mandatory = $true)]
    [string] $Revoke,
    # Default: %ProgramData%\sqail\service, or sqail2\service when that is
    # where an earlier sqail2 install keeps its data.
    [string] $DataDir = $(if (-not (Test-Path (Join-Path $env:ProgramData 'sqail\service')) -and
                              (Test-Path (Join-Path $env:ProgramData 'sqail2\service'))) {
                            Join-Path $env:ProgramData 'sqail2\service' } else {
                            Join-Path $env:ProgramData 'sqail\service' })
)

$ErrorActionPreference = 'Stop'

function Find-ServiceExe {
    $svc = Get-CimInstance Win32_Service -Filter "Name='sqail-service'" -ErrorAction SilentlyContinue
    if ($svc -and $svc.PathName -match '^"([^"]+)"') { return $Matches[1] }
    foreach ($p in (Join-Path (Split-Path -Parent $PSScriptRoot) 'sqail-service.exe'),
                   (Join-Path $env:ProgramFiles 'sqail\sqail-service.exe')) {
        if (Test-Path $p) { return $p }
    }
    throw 'sqail-service.exe not found'
}

if (-not (Test-Path (Join-Path $DataDir 'service.db'))) {
    Write-Host "No service data in $DataDir. Run this elevated on the service host (or pass -DataDir)." -ForegroundColor Red
    exit 1
}
$exe = Find-ServiceExe

switch ($PSCmdlet.ParameterSetName) {
    'List' { & $exe --data-dir $DataDir token list; exit $LASTEXITCODE }
    'Revoke' { & $exe --data-dir $DataDir token revoke $Revoke; exit $LASTEXITCODE }
}

$token = & $exe --data-dir $DataDir token create --name $Name --scope $Scope
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$fingerprint = & $exe --data-dir $DataDir fingerprint
$port = 7443
$cfg = Join-Path $DataDir 'sqail-service.toml'
if ((Test-Path $cfg) -and ((Get-Content $cfg -Raw) -match 'bind\s*=\s*"[^"]*:(\d+)"')) { $port = [int]$Matches[1] }

Write-Host ''
Write-Host "Token for $Name ($Scope scope). It is shown only now:" -ForegroundColor Cyan
Write-Host ''
Write-Host "    $token"
Write-Host ''
Write-Host 'In sqail: Service > Connect to a service...'
Write-Host "    URL          https://$([Net.Dns]::GetHostEntry('').HostName):$port"
Write-Host "    Token        (above)"
Write-Host "    Fingerprint  $fingerprint   (compare when sqail asks)"
