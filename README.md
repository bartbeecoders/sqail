# SQaiL

A lightweight, cross-platform desktop SQL database editor with AI integration. Built with Tauri v2.

Supports PostgreSQL, MySQL, SQLite, and Microsoft SQL Server.

## Prerequisites

All platforms require:

- [Rust](https://rustup.rs) (1.77.2+)
- [Node.js](https://nodejs.org) (20+)
- [pnpm](https://pnpm.io) (9+)

### Linux (Debian/Ubuntu)

```bash
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
```

### Linux (Fedora)

```bash
sudo dnf install webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel openssl-devel
```

### Linux (Arch)

```bash
sudo pacman -S webkit2gtk-4.1 libappindicator-gtk3 librsvg openssl
```

### macOS

```bash
xcode-select --install
```

### Windows

Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the "Desktop development with C++" workload. WebView2 is included in Windows 10 (1803+) and Windows 11.

## Development

```bash
pnpm install
pnpm tauri dev
```

Or use the helper script:

```bash
./scripts/run.sh dev
```

Note: On Linux, if you see a blank screen, the `run.sh` script sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` to work around a WebKitGTK DMA-BUF issue with some GPU drivers.

## Building for Release

Each build script checks prerequisites, builds the frontend and Rust backend, and produces distributable outputs.

### Linux

```bash
./scripts/build-linux.sh
```

**Output:** `src-tauri/target/release/bundle/appimage/sqlail_<version>_amd64.AppImage`

The AppImage is a single portable executable:

```bash
chmod +x sqlail_*.AppImage
./sqlail_*.AppImage
```

Also produces `.deb` and `.rpm` packages for system installation.

### macOS

```bash
./scripts/build-macos.sh
```

**Output:** `src-tauri/target/release/bundle/dmg/sqlail_<version>_aarch64.dmg`

Open the `.dmg` and drag the app to Applications. Also produces a `.app` bundle directly.

### Windows

```powershell
.\scripts\build-windows.ps1
```

**Output:** `src-tauri\target\release\bundle\nsis\sqlail_<version>_x64-setup.exe`

The NSIS `.exe` is a single-file installer. Run it to install. Also produces an `.msi` installer.

## Linting and Type Checking

```bash
pnpm check        # TypeScript type check
pnpm lint         # ESLint
./scripts/run.sh check  # All checks including cargo clippy
```

## Project Structure

```
src/              React frontend (TypeScript, Vite, Tailwind CSS)
src-tauri/        Rust backend (Tauri v2, sqlx, tiberius)
scripts/          Build and development helper scripts
Vibecoding/       Planning and architecture documents
```

## License

MIT
