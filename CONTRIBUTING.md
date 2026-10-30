# Contributing

sqail is a native SQL editor (Rust, egui) plus sqail-service, the HTTPS
gateway that holds the database connections. [CLAUDE.md](CLAUDE.md) is the
short guide to the workspace, the commands and the conventions; the
[README](README.md) has the longer version.

## How work flows

1. Every change starts from a GitHub issue (bug report or feature request
   forms). Issues labelled `claude` are picked up by Claude Code.
2. One branch per issue, from `main`: `fix/<issue>-<slug>` or
   `feat/<issue>-<slug>`.
3. Open a pull request that says `Closes #<issue>`. The template has the
   checklist.
4. `main` only accepts pull requests whose checks passed: build, clippy and
   tests on Linux and Windows, `cargo deny`/`cargo audit`, the integration
   tests against PostgreSQL and SQL Server, the fuzzer, the MSRV check.
5. Releases are tags on `main`; see *Releasing* in the README.

## Before you push

```bash
scripts/check.sh        # fmt, clippy -D warnings, tests, cargo-deny/audit
scripts/check.sh --it   # also the integration tests (podman databases)
```

- Commit messages: `<area>: <imperative summary>`, lowercase, no trailing
  period, at most 72 characters (`editor: close the popup after a space`).
- User-visible changes get a line in `CHANGELOG.md` under *Unreleased* and,
  where it applies, an update to `docs/user-guide.md`.
- No `unsafe`; new dependencies must pass `cargo deny` and `cargo audit`.
- `sqail-legacy/` is frozen: don't change it unless the issue asks for it.

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).
