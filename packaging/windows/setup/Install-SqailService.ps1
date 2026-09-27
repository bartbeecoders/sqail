<#
.SYNOPSIS
    Install sqail-service as a Windows service (run as Administrator).

.DESCRIPTION
    Installs and starts the service with safe defaults, then opens its admin
    page in your browser, already signed in. Everything else is set up
    there: who may connect (network), the certificate, SQLite folders,
    limits, tokens and database connections.

    1. Copies sqail-service.exe and sqail.exe to -InstallDir (skipped when
       they are already there, e.g. after the MSI).
    2. Registers and starts the "sqail-service" Windows service (LocalService,
       automatic start, restarts after a crash). The data directory is locked
       down to SYSTEM, Administrators and the service account.
    3. Adds a Windows Firewall rule for sqail-service.exe (Domain and Private
       networks, -FirewallRemoteAddress). It has no effect while the service
       listens on this computer only, which is the default; it is what lets
       other PCs in once you choose "Other computers too" on the admin page.
    4. Waits for the service and opens https://127.0.0.1:7443/admin/ signed
       in with the first admin token.

    Works in Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
    .\Install-SqailService.ps1
.EXAMPLE
    .\Install-SqailService.ps1 -NoBrowser -NoFirewall
#>
[CmdletBinding()]
param(
    [string] $InstallDir = (Join-Path $env:ProgramFiles 'sqail'),
    # Default: %ProgramData%\sqail\service, or sqail2\service when that is
    # where an earlier sqail2 install keeps its data.
    [string] $DataDir = $(if (-not (Test-Path (Join-Path $env:ProgramData 'sqail\service')) -and
                              (Test-Path (Join-Path $env:ProgramData 'sqail2\service'))) {
                            Join-Path $env:ProgramData 'sqail2\service' } else {
                            Join-Path $env:ProgramData 'sqail\service' }),
    # Who may reach the service once it serves other PCs (Windows Firewall
    # syntax: LocalSubnet, Any, 10.0.0.0/8, 192.168.1.10, ...).
    [string[]] $FirewallRemoteAddress = @('LocalSubnet'),
    # Do not add the firewall rule.
    [switch] $NoFirewall,
    # Do not open the admin page; print the sign-in link instead.
    [switch] $NoBrowser,
    # Register the service without starting it.
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'
$ServiceName = 'sqail-service'
$FirewallRule = 'sqail-service'

function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Done($m) { Write-Host " ok $m" -ForegroundColor Green }
function Fail($m) { Write-Host "err $m" -ForegroundColor Red; exit 1 }

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run this from an elevated PowerShell ("Run as administrator").'
}
if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
    Fail "The $ServiceName service is already installed. Manage it at https://127.0.0.1:7443/admin/ (or run Uninstall-SqailService.ps1 first; your data is kept)."
}

# --- 1. binaries -----------------------------------------------------------
$PackageDir = Split-Path -Parent $PSScriptRoot
$SourceExe = Join-Path $PackageDir 'sqail-service.exe'
if (-not (Test-Path $SourceExe)) { Fail "sqail-service.exe not found next to the setup folder ($PackageDir)." }
$Exe = Join-Path $InstallDir 'sqail-service.exe'
if ((Resolve-Path $PackageDir).Path.TrimEnd('\') -ne [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')) {
    Step "copying binaries to $InstallDir"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($f in 'sqail-service.exe', 'sqail.exe', 'LICENSE.txt') {
        $src = Join-Path $PackageDir $f
        if (Test-Path $src) { Copy-Item $src $InstallDir -Force }
    }
    $setupTarget = Join-Path $InstallDir 'setup'
    New-Item -ItemType Directory -Force -Path $setupTarget | Out-Null
    Copy-Item (Join-Path $PSScriptRoot '*') $setupTarget -Force
    Done 'binaries installed'
}

# --- 2. the Windows service ---------------------------------------------
Step 'registering the Windows service'
$installArgs = @('--data-dir', $DataDir, 'service', 'install')
if ($NoStart) { $installArgs += '--no-start' }
$output = & $Exe @installArgs
if ($LASTEXITCODE -ne 0) { Fail "sqail-service service install failed ($LASTEXITCODE)" }
# The output holds the one-time admin token; show it, and keep the link.
$output | ForEach-Object { Write-Host $_ }
$link = ($output | Select-String -Pattern 'https://\S+/admin/#token=\S+' | Select-Object -First 1).Matches.Value

# --- 3. firewall ------------------------------------------------------------
if (-not $NoFirewall) {
    Step "allowing sqail-service.exe through Windows Firewall ($($FirewallRemoteAddress -join ', '))"
    Get-NetFirewallRule -DisplayName $FirewallRule -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    # Tied to the program, not a port, so a port change on the admin page needs no new rule.
    New-NetFirewallRule -DisplayName $FirewallRule -Direction Inbound -Action Allow `
        -Protocol TCP -Program $Exe -RemoteAddress $FirewallRemoteAddress `
        -Profile Domain, Private | Out-Null
    Done 'firewall rule added (Domain and Private networks)'
}

# --- 4. check and open the admin page -------------------------------------
if ($NoStart) {
    Write-Host ''
    Write-Host "Start it with: Start-Service $ServiceName, then open the link above."
    exit 0
}
Step 'waiting for the service'
$ok = $false
for ($i = 0; $i -lt 30 -and -not $ok; $i++) {
    Start-Sleep -Milliseconds 500
    $health = & curl.exe -sk --max-time 2 'https://127.0.0.1:7443/v1/health' 2>$null
    $ok = ($LASTEXITCODE -eq 0 -and "$health" -match '"ok"')
}
if (-not $ok) { Fail "no answer on https://127.0.0.1:7443 - see $DataDir\logs\sqail-service.log" }
Done 'service is up'

Write-Host ''
Write-Host 'Next: finish the setup on the admin page' -ForegroundColor Cyan
Write-Host '  Your browser warns about the certificate once: the service uses a self-signed'
Write-Host '  certificate until you add your own (Settings > Certificate). Continue to the page.'
if ($link) {
    Write-Host "  Admin page (signs you in): $link"
    if (-not $NoBrowser) { Start-Process $link }
} else {
    Write-Host '  Admin page: https://127.0.0.1:7443/admin/  (sign in with an admin token)'
}
Write-Host '  Guide: SETUP.md'
