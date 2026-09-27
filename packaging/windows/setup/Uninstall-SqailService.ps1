<#
.SYNOPSIS
    Stop and remove the sqail-service Windows service (run as Administrator).

.DESCRIPTION
    Removes the service and its firewall rules. The data directory (tokens,
    connection profiles, audit log, master key) is kept unless -RemoveData is
    given; reinstalling with the same -DataDir picks everything up again.
#>
[CmdletBinding()]
param(
    [string] $DataDir = (Join-Path $env:ProgramData 'sqail2\service'),
    # Also delete the data directory. Stored database passwords are lost.
    [switch] $RemoveData
)

$ErrorActionPreference = 'Stop'
function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Done($m) { Write-Host " ok $m" -ForegroundColor Green }
function Fail($m) { Write-Host "err $m" -ForegroundColor Red; exit 1 }

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run this from an elevated PowerShell ("Run as administrator").'
}

$svc = Get-CimInstance Win32_Service -Filter "Name='sqail-service'" -ErrorAction SilentlyContinue
if ($svc) {
    # PathName looks like: "C:\Program Files\sqail2\sqail-service.exe" --data-dir ...
    $exe = if ($svc.PathName -match '^"([^"]+)"') { $Matches[1] } else { ($svc.PathName -split ' ')[0] }
    Step 'removing the Windows service'
    & $exe service uninstall
    if ($LASTEXITCODE -ne 0) { Fail "service uninstall failed ($LASTEXITCODE)" }
} else {
    Step 'the sqail-service service is not installed'
}

# 'sqail-service' is the current rule; 'sqail-service (TCP <port>)' came from older installers.
foreach ($rule in 'sqail-service', 'sqail-service (TCP *)') {
    Get-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue | Remove-NetFirewallRule
}
Done 'firewall rules removed'

if ($RemoveData) {
    if (Test-Path $DataDir) {
        Remove-Item -Recurse -Force $DataDir
        Done "deleted $DataDir"
    }
} else {
    Write-Host "Data kept in $DataDir (use -RemoveData to delete it)."
}
