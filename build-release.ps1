# Builds target\release\orbit-start.exe without local folder names inside it.
# (Rust records source paths for error messages; these flags rewrite them.)
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$rustup = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
$env:RUSTFLAGS = "--remap-path-prefix=$PSScriptRoot=orbit-start --remap-path-prefix=$cargoHome=cargo --remap-path-prefix=$rustup=rustup"
cargo build --release
