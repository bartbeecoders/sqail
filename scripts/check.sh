#!/usr/bin/env bash
# The gate every plan step must pass: format, lint, build, unit tests.
# Add --it to also run integration tests against the podman databases.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
info "cargo fmt";    cargo fmt --all -- --check
info "cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
info "cargo test";   cargo test --workspace
if command -v cargo-deny >/dev/null; then
    info "cargo deny";  cargo deny check --hide-inclusion-graph
else
    printf '    (cargo-deny not installed: cargo install --locked cargo-deny)\n'
fi
if command -v cargo-audit >/dev/null; then
    info "cargo audit"; cargo audit --quiet
else
    printf '    (cargo-audit not installed: cargo install --locked cargo-audit)\n'
fi
if [[ "${1:-}" == "--it" ]]; then
    "$ROOT/scripts/db.sh" up
    info "integration tests"; SQAIL_IT=1 cargo test --workspace -- --ignored
fi
ok "all checks passed"
