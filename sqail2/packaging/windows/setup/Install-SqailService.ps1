<#
.SYNOPSIS
    Install sqail-service as a Windows service (run as Administrator).

.DESCRIPTION
    1. Copies sqail-service.exe and sqail2.exe to -InstallDir (skipped when
       they are already there, e.g. after the MSI).
    2. Writes <DataDir>\sqail-service.toml (listen address, certificate,
       SQLite folders) unless it exists (use -Force to overwrite).
    3. Registers and starts the "sqail-service" Windows service. This
       locks the data directory down to SYSTEM, Administrators and the
       service account, and prints the first admin token (once).
    4. With -Network: opens the port in Windows Firewall.
    5. Checks the service answers and prints the certificate fingerprint.

    Works in Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
    .\Install-SqailService.ps1
    Local only (https://127.0.0.1:7443), self-signed certificate.

.EXAMPLE
    .\Install-SqailService.ps1 -Network -CertFile C:\certs\gw.pem -KeyFile C:\certs\gw.key
    Shared gateway on all interfaces with your own certificate; firewall
    opened for the local subnet.
#>
[CmdletBinding()]
param(
    [string] $InstallDir = (Join-Path $env:ProgramFiles 'sqail2'),
    [string] $DataDir = (Join-Path $env:ProgramData 'sqail2\service'),
    # Listen on all interfaces (default: this machine only).
    [switch] $Network,
    [int] $Port = 7443,
    # PEM certificate chain and private key. Default: self-signed certificate.
    [string] $CertFile,
    [string] $KeyFile,
    # Who may reach the port when -Network is used (Windows Firewall syntax:
    # LocalSubnet, Any, 10.0.0.0/8, 192.168.1.10, ...).
    [string[]] $FirewallRemoteAddress = @('LocalSubnet'),
    # Folders SQLite profiles may open (none: SQLite disabled).
    [string[]] $SqliteFolder = @(),
    # Rewrite sqail-service.toml even if it exists.
    [switch] $Force,
    # Register the service without starting it.
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'
$ServiceName = 'sqail-service'
$FirewallRule = "sqail-service (TCP $Port)"

function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Done($m) { Write-Host " ok $m" -ForegroundColor Green }
function Fail($m) { Write-Host "err $m" -ForegroundColor Red; exit 1 }

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run this from an elevated PowerShell ("Run as administrator").'
}
if (($CertFile -and -not $KeyFile) -or ($KeyFile -and -not $CertFile)) {
    Fail '-CertFile and -KeyFile go together.'
}
if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
    Fail "The $ServiceName service is already installed. Run Uninstall-SqailService.ps1 first (your data is kept)."
}

# --- 1. binaries -----------------------------------------------------------
$PackageDir = Split-Path -Parent $PSScriptRoot
$SourceExe = Join-Path $PackageDir 'sqail-service.exe'
if (-not (Test-Path $SourceExe)) { Fail "sqail-service.exe not found next to the setup folder ($PackageDir)." }
$Exe = Join-Path $InstallDir 'sqail-service.exe'
if ((Resolve-Path $PackageDir).Path.TrimEnd('\') -ne [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')) {
    Step "copying binaries to $InstallDir"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($f in 'sqail-service.exe', 'sqail2.exe', 'LICENSE.txt') {
        $src = Join-Path $PackageDir $f
        if (Test-Path $src) { Copy-Item $src $InstallDir -Force }
    }
    $setupTarget = Join-Path $InstallDir 'setup'
    New-Item -ItemType Directory -Force -Path $setupTarget | Out-Null
    Copy-Item (Join-Path $PSScriptRoot '*') $setupTarget -Force
    Done 'binaries installed'
}

# --- 2. configuration -----------------------------------------------------
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
$ConfigPath = Join-Path $DataDir 'sqail-service.toml'
if ((Test-Path $ConfigPath) -and -not $Force) {
    Step "keeping existing $ConfigPath (-Force rewrites it)"
} else {
    Step "writing $ConfigPath"
    $bindHost = '127.0.0.1'
    if ($Network) { $bindHost = '0.0.0.0' }
    # TOML literal strings ('...') need no escaping of backslashes.
    $lines = @(
        '# Written by Install-SqailService.ps1. Every key: sqail-service.example.toml',
        "bind = `"$($bindHost):$Port`"",
        'log_format = "json"',
        '',
        '[tls]'
    )
    if ($CertFile) {
        # Keep copies inside the data dir, which the service account can read.
        $tlsDir = Join-Path $DataDir 'tls'
        New-Item -ItemType Directory -Force -Path $tlsDir | Out-Null
        Copy-Item $CertFile (Join-Path $tlsDir 'server-cert.pem') -Force
        Copy-Item $KeyFile (Join-Path $tlsDir 'server-key.pem') -Force
        $lines += "cert = '$(Join-Path $tlsDir 'server-cert.pem')'"
        $lines += "key = '$(Join-Path $tlsDir 'server-key.pem')'"
    } else {
        $lines += '# Self-signed certificate in tls\dev-cert.pem (clients pin its fingerprint).'
    }
    $lines += ''
    $lines += '[sqlite]'
    $dirs = ($SqliteFolder | ForEach-Object { "'$((Resolve-Path $_).Path)'" }) -join ', '
    $lines += "allowed_dirs = [$dirs]"
    [IO.File]::WriteAllText($ConfigPath, (($lines -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding $false))
    Done 'configuration written'
}
if ($SqliteFolder.Count -gt 0) {
    # The service runs as LocalService: let it use the SQLite folders.
    foreach ($d in $SqliteFolder) {
        & icacls.exe $d /grant '*S-1-5-19:(OI)(CI)M' /Q | Out-Null
    }
}

# --- 3. the Windows service ---------------------------------------------
Step 'registering the Windows service'
$installArgs = @('--data-dir', $DataDir, 'service', 'install')
if ($NoStart) { $installArgs += '--no-start' }
& $Exe @installArgs
if ($LASTEXITCODE -ne 0) { Fail "sqail-service service install failed ($LASTEXITCODE)" }

# --- 4. firewall ------------------------------------------------------------
if ($Network) {
    Step "opening TCP $Port in Windows Firewall for $($FirewallRemoteAddress -join ', ')"
    Get-NetFirewallRule -DisplayName $FirewallRule -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $FirewallRule -Direction Inbound -Action Allow `
        -Protocol TCP -LocalPort $Port -Program $Exe -RemoteAddress $FirewallRemoteAddress `
        -Profile Domain, Private | Out-Null
    Done 'firewall rule added (Domain and Private networks)'
}

# --- 5. check ---------------------------------------------------------------
if (-not $NoStart) {
    Step 'waiting for the service'
    $ok = $false
    for ($i = 0; $i -lt 30 -and -not $ok; $i++) {
        Start-Sleep -Milliseconds 500
        $health = & curl.exe -sk --max-time 2 "https://127.0.0.1:$Port/v1/health" 2>$null
        $ok = ($LASTEXITCODE -eq 0 -and "$health" -match '"ok"')
    }
    if (-not $ok) { Fail "no answer on https://127.0.0.1:$Port - see $DataDir\logs\sqail-service.log" }
    Done "service is up: $health"
}
$fingerprint = & $Exe --data-dir $DataDir fingerprint

$name = [Net.Dns]::GetHostEntry('').HostName
Write-Host ''
Write-Host 'Next steps' -ForegroundColor Cyan
Write-Host "  URL for sqail2:       https://$(if ($Network) { $name } else { '127.0.0.1' }):$Port"
Write-Host "  Certificate SHA-256:  $fingerprint"
Write-Host '  Give users a token:   .\New-SqailToken.ps1 -Name alice'
Write-Host '  Add a SQL Server:     .\Add-SqailSqlServer.ps1 -Name "Sales (prod)" -Server SQL01 -Database Sales -User sqail_reader'
Write-Host '  Guide:                SETUP.md'
