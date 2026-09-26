# Run the sqail2 desktop editor.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
cargo run -p sqail-ui --bin sqail2 -- @args
