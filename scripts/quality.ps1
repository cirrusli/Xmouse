[CmdletBinding()]
param(
    [switch]$ClipboardRoundTrip,
    [switch]$Coverage
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$workspaceCargo = Join-Path $projectRoot 'work/cargo/bin/cargo.exe'
$installedCargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($null -ne $installedCargo) {
    $cargo = $installedCargo.Source
}
elseif (Test-Path -LiteralPath $workspaceCargo) {
    $cargo = $workspaceCargo
}
else {
    throw 'Cargo was not found in PATH or work/cargo/bin. Install Rust stable first.'
}

if ((Test-Path -LiteralPath $workspaceCargo) -and $cargo -eq $workspaceCargo) {
    $env:CARGO_HOME = Join-Path $projectRoot 'work/cargo-home'
    $env:RUSTUP_HOME = Join-Path $projectRoot 'work/rustup'
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'
}
if (Test-Path -LiteralPath (Join-Path $projectRoot 'work/sqlite')) {
    $env:LIBRARY_PATH = Join-Path $projectRoot 'work/sqlite'
}
$localPaths = @(
    (Join-Path $projectRoot 'work/cargo-install-home/bin'),
    (Join-Path $projectRoot 'work/cargo/bin'),
    (Join-Path $projectRoot 'work/icon-preprocessor'),
    (Join-Path $projectRoot 'work/binutils/mingw64/bin'),
    (Join-Path $projectRoot 'work/sqlite')
) | Where-Object { Test-Path -LiteralPath $_ }
$env:PATH = ($localPaths + $env:PATH) -join ';'

Push-Location $projectRoot
try {
    & (Join-Path $PSScriptRoot 'verify-test-baseline.ps1')

    & $cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed.' }

    & $cargo clippy --release --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed.' }

    & $cargo test --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed.' }

    if ($ClipboardRoundTrip) {
        & $cargo test --release materialized_snapshot_round_trips_text_and_custom_formats -- --ignored
        if ($LASTEXITCODE -ne 0) { throw 'Interactive clipboard round-trip test failed.' }
    }

    # Tests produce a separate harness binary. Build the shipping executable
    # explicitly so target/release/xmouse.exe can never remain stale.
    & $cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed.' }

    if ($Coverage) {
        & $cargo llvm-cov --summary-only --fail-under-lines 24
        if ($LASTEXITCODE -ne 0) {
            throw 'Coverage failed. Install cargo-llvm-cov and the llvm-tools-preview Rust component.'
        }
    }
}
finally {
    Pop-Location
}
