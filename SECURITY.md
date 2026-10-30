# Security policy

## Reporting a vulnerability

Please report security problems privately through
[GitHub's private vulnerability reporting](https://github.com/bartbeecoders/sqail/security/advisories/new)
(repository → *Security* → *Report a vulnerability*), not in a public issue.
Include the version, the engine and platform, and the steps or SQL that
show the problem.

Fixes ship as a new release; the advisory is published once that release
is out.

## Supported versions

Only the latest release of sqail and sqail-service gets security fixes.
The 0.x Tauri app in `sqail-legacy/` is frozen and unsupported.

## Threat model

[docs/security.md](docs/security.md) describes what sqail-service protects
against, how, and which tests cover it.
