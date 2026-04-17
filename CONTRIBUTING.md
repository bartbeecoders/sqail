# Contributing to sqail

Thanks for considering a contribution. sqail is a small, open-source project and every bit of help — code, docs, bug reports, screenshots, migration guides — is welcome.

This document explains how to get set up, how we work, and what makes a contribution easy to accept.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By participating you agree to uphold it. Report unacceptable behavior to the maintainers through a direct message or a private channel — not a public issue.

## Where things live

- **Canonical source** — [Codeberg](https://codeberg.org/bartbeecoders/sqail)
- **Mirror** — [GitHub](https://github.com/bartbeecoders/sqail)
- **Website** — [sqail.io](https://sqail.io)
- **Issues** — file them on GitHub (primary) or Codeberg — both are watched
- **Discussions** — GitHub Discussions for questions, ideas, and show-and-tell

If in doubt, open an issue on GitHub. We'll mirror or move it if needed.

## Ways to contribute

### Report a bug

A good bug report usually has:

1. What you did (exact steps, ideally a minimal SQL query or screenshot)
2. What you expected
3. What actually happened (include the error text, not just "it broke")
4. Your OS + sqail version (visible in the About dialog)
5. Database driver + version if the bug is query-related

Before filing: search existing issues. If you find a related one, add your details there instead of opening a new one.

### Suggest a feature

Open an issue tagged `enhancement`. Describe the problem first, the proposed solution second. We'd rather understand *why* you need something than debate *how* to build it. Features that reduce complexity beat features that add it.

### Improve documentation

Documentation changes are the easiest PRs to land. Typo fixes, clearer install instructions, better examples, migration guides from other tools — all welcome. Don't wait for permission.

### Write code

See the [Development](#development) section below.

## Development

### Prerequisites

See the [Prerequisites section of the README](README.md#prerequisites) for platform-specific tooling.

### First-time setup

```bash
git clone https://codeberg.org/bartbeecoders/sqail.git
cd sqail
pnpm install
pnpm tauri dev
```

On Linux, if you see a blank window, run `./scripts/run.sh dev` — it sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` to work around a WebKitGTK DMA-BUF issue on some GPU drivers.

### Project layout

```
src/              React frontend (TypeScript, Vite, Tailwind)
src-tauri/        Rust backend (Tauri v2, sqlx, tiberius)
sqail.portal/     Marketing website (separate Vite app)
scripts/          Build + dev helper scripts
Vibecoding/       Design notes and planning docs
marketing/        Brand guide, press kit, marketing strategy/plan
```

### Checks before you push

```bash
pnpm check        # TypeScript
pnpm lint         # ESLint
./scripts/run.sh check   # Full check including cargo clippy
```

If any of these fail on main, that's a bug — please flag it.

## Pull requests

### What makes a PR easy to accept

- **Small and focused.** One logical change per PR. Two bug fixes = two PRs.
- **Explains the why.** A 3-line description is plenty. "Fixes X because Y was happening when Z" beats "misc improvements".
- **Doesn't drift.** If you opened a PR to fix a bug, don't also rename variables, reformat unrelated files, or refactor helpers. Those are separate PRs.
- **Passes the checks.** Type-check, lint, and clippy all clean before you mark ready for review.
- **Links to an issue** if one exists. `Fixes #123` in the description auto-closes the issue on merge.

### What tends to get pushback

- Drive-by style changes to code you weren't touching
- New dependencies without a stated reason
- Features that don't match the current roadmap (talk to us in an issue first — saves rework)
- Commits with generated messages that don't describe the change

### Commit messages

No strict convention, but aim for:

```
<area>: <imperative summary of the change>

<optional body — explain why if it's not obvious>
```

Examples from the existing history:

```
v0.3.5: AI prompt history, editor context for AI, split editor sharing same file
add JWT authentication to DbService, remove AllowAnonymous from all endpoints
```

Present tense, lowercase start, no trailing period. Keep the first line under ~72 chars.

## Adding a database driver

Driver support lives in `src-tauri/` and uses `sqlx` (Postgres, MySQL, SQLite) or `tiberius` (SQL Server). If you want to add a new driver:

1. Open an issue first — we want to make sure the driver fits the binary-size budget (<20 MB total)
2. Follow the pattern in the existing driver modules
3. Add connection form fields in the React side under `src/components/`
4. Add a smoke test against a real database, not a mock

## Working on the inline AI sidecar

The inline AI feature (see `Vibecoding/inline-ai.md`) runs a local `llama-server` process next to the Tauri app. For development you build that binary once from source:

```bash
./scripts/fetch-llama-cpp.sh          # clones + builds llama.cpp with CUDA
./scripts/fetch-inline-models.sh      # downloads the three catalog models (~16 GB total)
```

Both scripts cache everything under `.cache/inline-ai/` (gitignored). The Rust sidecar resolver finds the dev-built binary automatically. If you prefer a different build, set `SQAIL_LLAMA_SERVER_PATH=/path/to/your/llama-server` before launching.

The CUDA build assumes you're on Linux x86_64 with CUDA 12.x + an NVIDIA GPU. For CPU-only or non-CUDA GPU development, edit the cmake flags in `scripts/fetch-llama-cpp.sh` (drop `-DGGML_CUDA=ON`) or pass a Vulkan prebuilt via `SQAIL_LLAMA_SERVER_PATH`.

Release bundling via Tauri's `externalBin` is staged in `scripts/fetch-llama-binaries.sh` — the Windows/macOS prebuilts are on the Phase G punch-list and currently TODO.

## Adding an AI provider

AI providers are pluggable. See the existing Claude/OpenAI/Minimax integrations for the pattern. New providers should:

1. Be OpenAI-compatible *or* bring a self-contained client
2. Support streaming responses if the provider's API allows it
3. Accept user-supplied API keys — never hardcode

## Releasing

Releases are cut by the maintainer. If you want to help:

- Make sure your changes have release notes in `RELEASES.md`
- Mention user-visible changes, not internal refactors
- One line per change is fine

## License

By submitting a contribution, you agree that your work will be released under the same license as the project (see [LICENSE](LICENSE)).

## Questions

Open a discussion on GitHub, ping the issue tracker, or reach the maintainer directly. No question is too small — if the docs weren't clear enough to answer it, that's a doc bug we want to fix.
