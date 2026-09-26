$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $ProjectRoot

# Cursor/VS Code terminals keep the PATH from when the editor launched, so a
# user-PATH install of pnpm is invisible until the editor restarts. Put known
# shim dirs on PATH, then fall back to Corepack.
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    $pnpmDirs = @(
        (Join-Path $env:LOCALAPPDATA "pnpm"),
        (Join-Path $env:APPDATA "npm"),
        (Join-Path $env:APPDATA "pnpm")
    )
    foreach ($dir in $pnpmDirs) {
        if ((Test-Path (Join-Path $dir "pnpm.cmd")) -or (Test-Path (Join-Path $dir "pnpm.ps1")) -or (Test-Path (Join-Path $dir "pnpm.exe"))) {
            $env:PATH = "$dir;$env:PATH"
        }
    }
}
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    if (-not (Get-Command corepack -ErrorAction SilentlyContinue)) {
        Write-Host "Error: pnpm not found, and Node/Corepack is not on PATH." -ForegroundColor Red
        Write-Host "Install Node.js 20+ from https://nodejs.org then re-run this script." -ForegroundColor Red
        exit 1
    }
    $shimDir = Join-Path $env:LOCALAPPDATA "pnpm"
    New-Item -ItemType Directory -Force -Path $shimDir | Out-Null
    corepack enable --install-directory $shimDir | Out-Null
    $env:PATH = "$shimDir;$env:PATH"
}
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Host "Error: pnpm not found. Install from https://pnpm.io/installation" -ForegroundColor Red
    exit 1
}

if (-not (Test-Path (Join-Path $ProjectRoot "node_modules"))) {
    Write-Host "node_modules missing — installing dependencies..."
    pnpm install
}

$Mode = if ($args.Count -gt 0) { $args[0] } else { "dev" }

switch ($Mode) {
    "dev" {
        Write-Host "Starting sqail in development mode..."
        pnpm tauri dev
    }
    "build" {
        Write-Host "Building sqail for release..."
        pnpm tauri build
    }
    "check" {
        Write-Host "Running all checks..."
        pnpm check
        pnpm lint
        Set-Location (Join-Path $ProjectRoot "src-tauri")
        cargo clippy -- -D warnings
        Set-Location $ProjectRoot
        Write-Host "All checks passed."
    }
    default {
        Write-Host "Usage: .\scripts\run.ps1 {dev|build|check}"
        Write-Host "  dev    - Run in development mode with hot reload (default)"
        Write-Host "  build  - Build release binary"
        Write-Host "  check  - Run tsc, eslint, and cargo clippy"
        exit 1
    }
}
