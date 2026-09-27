# Run the sqail desktop editor.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
cargo run -p sqail-ui --bin sqail -- @args
