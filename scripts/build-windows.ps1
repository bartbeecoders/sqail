$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $ProjectRoot

Write-Host "=== Building sqail for Windows ===" -ForegroundColor Cyan

# Check prerequisites
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Error: Rust/Cargo not found. Install from https://rustup.rs" -ForegroundColor Red
    exit 1
}
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Host "Error: pnpm not found. Install from https://pnpm.io" -ForegroundColor Red
    exit 1
}

# Check for Visual Studio Build Tools / MSVC
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vsWhere)) {
        Write-Host "Warning: Visual Studio Build Tools may not be installed." -ForegroundColor Yellow
        Write-Host "Install from: https://visualstudio.microsoft.com/visual-cpp-build-tools/" -ForegroundColor Yellow
    }
}

# Install frontend dependencies
Write-Host "Installing frontend dependencies..."
pnpm install --frozen-lockfile

# Build
Write-Host "Building release..."
pnpm tauri build

# Show output
$BundleDir = Join-Path $ProjectRoot "src-tauri\target\release\bundle"
Write-Host ""
Write-Host "=== Build complete ===" -ForegroundColor Green
Write-Host "Outputs:"

$NsisDir = Join-Path $BundleDir "nsis"
if (Test-Path $NsisDir) {
    Write-Host "  NSIS installer (single .exe):"
    Get-ChildItem "$NsisDir\*.exe" | ForEach-Object { Write-Host "    $($_.FullName) ($([math]::Round($_.Length / 1MB, 1)) MB)" }
}

$MsiDir = Join-Path $BundleDir "msi"
if (Test-Path $MsiDir) {
    Write-Host "  MSI installer:"
    Get-ChildItem "$MsiDir\*.msi" | ForEach-Object { Write-Host "    $($_.FullName) ($([math]::Round($_.Length / 1MB, 1)) MB)" }
}

Write-Host ""
Write-Host "The NSIS .exe is a single-file installer. Distribute it directly."
