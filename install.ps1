$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Prefix = if ($env:HOLOGRAM_INSTALL_PREFIX) { $env:HOLOGRAM_INSTALL_PREFIX } else { Join-Path $HOME ".local" }
$DestinationDir = Join-Path $Prefix "bin"
$Destination = Join-Path $DestinationDir "hologram.exe"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo is required to build this source distribution. Install Rust from https://rustup.rs first."
}

cargo build --manifest-path (Join-Path $Root "Cargo.toml") --release --locked --package hologram-live --bin hologram
New-Item -ItemType Directory -Force -Path $DestinationDir | Out-Null
Copy-Item -Force (Join-Path $Root "target\release\hologram.exe") $Destination
& $Destination init 2>$null | Out-Null
Write-Host "installed hologram to $Destination"
