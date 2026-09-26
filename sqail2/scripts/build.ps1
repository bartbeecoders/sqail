# Build everything. Pass --release for an optimised build.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
Invoke-Checked cargo build --workspace @args
Ok 'build finished'
