# Run sqail-service (the HTTPS REST gateway). Extra args go to the service.
. "$PSScriptRoot/common.ps1"
Set-Location $Root
cargo run -p sqail-service -- @args
