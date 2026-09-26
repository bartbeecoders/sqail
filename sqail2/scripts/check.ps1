# The gate every plan step must pass: format, lint, build, unit tests.
# Add -It to also run integration tests against the podman databases.
param([switch]$It)
. "$PSScriptRoot/common.ps1"
Set-Location $Root
Info 'cargo fmt';    Invoke-Checked cargo fmt --all -- --check
Info 'cargo clippy'; Invoke-Checked cargo clippy --workspace --all-targets -- -D warnings
Info 'cargo test';   Invoke-Checked cargo test --workspace
if (Get-Command cargo-deny -ErrorAction SilentlyContinue) { Info 'cargo deny'; Invoke-Checked cargo deny check --hide-inclusion-graph }
else { Write-Host '    (cargo-deny not installed: cargo install --locked cargo-deny)' }
if (Get-Command cargo-audit -ErrorAction SilentlyContinue) { Info 'cargo audit'; Invoke-Checked cargo audit --quiet }
else { Write-Host '    (cargo-audit not installed: cargo install --locked cargo-audit)' }
if ($It) {
    & "$PSScriptRoot/db.ps1" up
    Info 'integration tests'; $env:SQAIL2_IT = '1'
    Invoke-Checked cargo test --workspace -- --ignored
}
Ok 'all checks passed'
