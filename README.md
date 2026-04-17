# sqail

**A fast, small, open-source SQL editor that makes AI-assisted querying feel native.**

sqail (pronounced *"snail"*) is a lightweight cross-platform desktop SQL client with first-class AI integration, built on Tauri v2. It launches in under a second, stays under 20 MB, and ships with natural-language-to-SQL, query explanation, and optimization out of the box — without telemetry, lock-in, or a paid tier.

It speaks PostgreSQL, MySQL, SQLite, and Microsoft SQL Server today. It uses the same Monaco editor that powers VS Code, stores credentials locally in encrypted SurrealDB, and lets you bring your own API key for Claude, OpenAI, Minimax, Z.ai, LM Studio, or any OpenAI-compatible endpoint. Everything else is up to you.

[**Download for your OS →**](https://sqail.io) &nbsp;·&nbsp; [Codeberg](https://codeberg.org/bartbeecoders/sqail) &nbsp;·&nbsp; [GitHub mirror](https://github.com/bartbeecoders/sqail)

## Why sqail

- **Fast** — sub-20 MB binary, sub-second launch. No Electron, no JVM, no Chromium bundle.
- **Smart** — schema-aware autocomplete and AI that actually knows your tables. NL-to-SQL, explain, optimize.
- **Free** — open source forever. No account, no freemium, no feature gates.
- **Private** — zero telemetry. Credentials stay on your machine. AI providers are only contacted when you configure them.
- **Universal** — Postgres, MySQL, SQLite, SQL Server, one editor.

## Features

- Monaco-based editor with multi-cursor, split view, snippets, and dark/light themes
- Tabbed workspace with query history you can search, filter, and re-run
- Schema browser, connection manager, and SSH tunnel support
- AI command palette: format, explain, optimize, generate SQL from natural language
- Multi-provider AI: Claude, OpenAI, Minimax, Z.ai, LM Studio, Claude Code CLI, OpenAI-compatible
- **Inline AI completion** — ghost-text SQL suggestions from a local llama.cpp sidecar, opt-in, no cloud calls
- Keyboard-first shortcuts, fully customizable
- Privacy-respecting: credentials in local encrypted SurrealDB, no telemetry

## Inline AI completion

Ghost-text SQL suggestions powered by a local [llama.cpp](https://github.com/ggml-org/llama.cpp) sidecar — no network round-trips, nothing leaves your machine. Toggle it on in **Settings → Inline AI**, pick a model, and it starts suggesting completions as you type. Press `Tab` to accept, `Esc` to dismiss.

Models are all Q4_K_M GGUF quants. Pick one from the catalog; it's downloaded once into your app-data dir and cached for life.

| Tier | Model | VRAM | Download | Notes |
| --- | --- | --- | --- | --- |
| Default | Qwen2.5-Coder-3B | 2.5 GB | 2 GB | Fits any GPU ≥ 4 GB. Great quality across all our benchmarks. |
| Performance | DeepSeek-Coder-V2-Lite (16B MoE) | 11 GB | 9.7 GB | Needs a 16 GB+ GPU. Slightly better quality, similar speed. |
| Low-end / CPU | Qwen2.5-Coder-1.5B | 1.6 GB | 1.1 GB | Runs fine on CPU (~15 tok/s) when no GPU is available. |

On a desktop-class NVIDIA GPU (RTX 4080 Super) the default model hits **~6 ms first-token latency** and **~200 tok/s** throughput — well under the budget for real-time ghost text. See [`Vibecoding/inline-ai-benchmarks.md`](./Vibecoding/inline-ai-benchmarks.md) for the full Phase A benchmark results, and [`Vibecoding/inline-ai.md`](./Vibecoding/inline-ai.md) for the architecture plan.

Feature is **off by default** — you have to flip the toggle. Nothing is downloaded or run until you do.

**v0.5.0 note:** prebuilt `llama-server` binaries for Windows/macOS are not yet bundled with the installer. Either point at an existing OpenAI-compatible local endpoint (Ollama, LM Studio) in the settings, or supply your own `llama-server` build via the `SQAIL_LLAMA_SERVER_PATH` env var. Linux users can run `./scripts/fetch-llama-cpp.sh` to build one with CUDA.

## Install

Prebuilt binaries for Linux, macOS, and Windows are available from [sqail.io](https://sqail.io). If you prefer to build from source, see [Development](#development).

### macOS note: Gatekeeper warnings on first launch

sqail is not yet signed with an Apple Developer ID, so macOS Gatekeeper will refuse to launch it after download with one of two misleading errors, depending on your macOS version:

- **"sqail is damaged and cannot be opened"** (older macOS)
- **"Apple could not verify 'sqail' is free of malware"** (Sonoma / Sequoia and later)

Both mean the same thing: the app has a quarantine attribute from being downloaded, and it isn't notarized. The app is fine — you just need to remove the quarantine attribute once. After dragging sqail to `/Applications`, run:

```bash
xattr -cr /Applications/sqail.app
```

Alternatively, open **System Settings → Privacy & Security**, scroll to the bottom, and click **"Open Anyway"** next to the sqail warning after your first failed launch attempt.

You only need to do this once per install. Proper signing + notarization is on the roadmap.

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
sqail.portal/     Marketing website (sqail.io)
scripts/          Build and development helper scripts
marketing/        Brand, strategy, and plan documents
Vibecoding/       Planning and architecture documents
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports, docs fixes, and migration guides from other tools are all welcome.

## License

MIT
