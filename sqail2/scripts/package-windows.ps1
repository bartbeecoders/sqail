<#
.SYNOPSIS
    Build the Windows release of sqail2 + sqail-service.

.DESCRIPTION
    Produces in dist\:
      sqail2-<ver>-windows-x64.zip   portable: editor, service, setup kit, docs
      sqail2-<ver>-windows-x64.msi   installer, when the WiX 5 CLI `wix` is on PATH
                                     (dotnet tool install --global wix --version 5.0.2;
                                     WiX 6+ needs its OSMF EULA accepted first)
      *.sha256                       checksums

    Layout of the zip (the MSI installs the same tree to Program Files\sqail2):
      sqail2.exe, sqail-service.exe
      SETUP.md                      how to set up the service and SQL Server
      README.txt, LICENSE.txt
      setup\                        Install-/Uninstall-SqailService, New-SqailToken,
                                    Add-SqailSqlServer (.ps1, plus .cmd launchers)
      docs\                         user guide, operations, API, security
      sqail-service.example.toml

    Run on Windows with the Rust MSVC toolchain. Works in Windows PowerShell 5.1
    and PowerShell 7.

.PARAMETER SkipBuild
    Package the binaries already in target\release.

.PARAMETER SignThumbprint
    Code-sign both executables (and the MSI) with this certificate from the
    current user's store, using signtool.exe from the Windows SDK.
#>
[CmdletBinding()]
param(
    [switch] $SkipBuild,
    [string] $SignThumbprint = $env:SQAIL_SIGN_THUMBPRINT
)
. "$PSScriptRoot/common.ps1"
Set-Location $Root

$Version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$Name = "sqail2-$Version-windows-x64"
$Dist = Join-Path $Root 'dist'
$Stage = Join-Path $Dist $Name

function Write-Checksum($File) {
    $hash = (Get-FileHash -Algorithm SHA256 $File).Hash.ToLower()
    "$hash  $(Split-Path -Leaf $File)" | Set-Content -Encoding ascii "$File.sha256"
}
function Invoke-Sign($File) {
    if (-not $SignThumbprint) { return }
    Need signtool 'install the Windows SDK (signtool.exe) or unset SQAIL_SIGN_THUMBPRINT'
    Invoke-Checked signtool sign /sha1 $SignThumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $File
}

if (-not $SkipBuild) {
    Need cargo 'install Rust from https://rustup.rs (MSVC toolchain)'
    Info 'building release binaries'
    Invoke-Checked cargo build --release --locked -p sqail-ui -p sqail-service
}
foreach ($exe in 'sqail2.exe', 'sqail-service.exe') {
    if (-not (Test-Path "target/release/$exe")) { Die "target/release/$exe missing (build without -SkipBuild)" }
}

Info "staging $Name"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force -Path $Stage, "$Stage/setup", "$Stage/docs" | Out-Null
Copy-Item target/release/sqail2.exe, target/release/sqail-service.exe $Stage
foreach ($exe in 'sqail2.exe', 'sqail-service.exe') { Invoke-Sign (Join-Path $Stage $exe) }
Copy-Item packaging/icons/sqail2.ico $Stage
Copy-Item LICENSE (Join-Path $Stage 'LICENSE.txt')
Copy-Item docs/windows-setup.md (Join-Path $Stage 'SETUP.md')
Copy-Item dev/sqail-service.example.toml $Stage
Copy-Item packaging/windows/setup/* "$Stage/setup"
foreach ($d in 'docs/user-guide.md', 'docs/windows-setup.md', 'docs/operations.md', 'docs/security.md', 'docs/api.md',
               'crates/sqail-ui/assets/fonts/Inter-OFL.txt', 'crates/sqail-ui/assets/fonts/JetBrainsMono-OFL.txt') {
    Copy-Item $d "$Stage/docs"
}
@"
sqail2 ${Version}: a fast SQL editor backed by sqail-service
=============================================================

Just me, on this PC
    Run sqail2.exe and choose "Use the local service". Then add a
    connection (+ New connection). Nothing else to install.

A shared gateway for a team (connects to your SQL Servers)
    Right-click setup\Install-SqailService.cmd > Run as administrator
    (add -Network to serve other PCs), then follow SETUP.md.

Files
    sqail2.exe              the editor
    sqail-service.exe       the HTTPS gateway (all database access goes through it)
    SETUP.md                step-by-step setup, SQL Server preparation, troubleshooting
    setup\                  install/uninstall the Windows service, create tokens,
                            register SQL Server databases
    docs\                   user guide, operations, REST API, security
"@ | Set-Content -Encoding ascii (Join-Path $Stage 'README.txt')

Info 'zipping'
$Zip = Join-Path $Dist "$Name.zip"
if (Test-Path $Zip) { Remove-Item $Zip }
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Checksum $Zip
Ok "dist/$Name.zip"

if (Get-Command wix -ErrorAction SilentlyContinue) {
    Info 'building MSI'
    $Msi = Join-Path $Dist "$Name.msi"
    Invoke-Checked wix build packaging/wix/sqail2.wxs -arch x64 `
        -d "Version=$Version" -d "StageDir=$Stage" -o $Msi
    Invoke-Sign $Msi
    Write-Checksum $Msi
    Ok "dist/$Name.msi"
} else {
    Info 'wix not found: skipping the MSI (dotnet tool install --global wix --version 5.0.2)'
}
