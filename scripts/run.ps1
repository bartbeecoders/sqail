$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $ProjectRoot

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
