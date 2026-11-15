#!/usr/bin/env bash
# Cross-compiles the Quadra Windows app (quadra.exe + NSIS installer) on a Linux x86_64 host.
# Idempotent: installs only what is missing. Tested on Ubuntu 24.04.
#   tools/build-windows-cross.sh
# Output: out/windows/quadra.exe, out/windows/Quadra_<version>_x64-setup.exe (+ sing-box.exe if built).
set -euo pipefail

TARGET=x86_64-pc-windows-msvc
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SINGBOX_SRC="${SINGBOX_SRC:-$REPO/../sing-box-xhttp}"
OUT="${OUT:-$REPO/out/windows}"
export PATH="$HOME/.cargo/bin:/opt/node/bin:/usr/local/go/bin:$PATH"
# cargo-xwin downloads the MSVC CRT and Windows SDK; this accepts Microsoft's licence for it.
export XWIN_ACCEPT_LICENSE=1

need() { command -v "$1" >/dev/null 2>&1; }
SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO=sudo

# 1. System packages: clang/lld/llvm (linker, llvm-rc), NSIS (installer), 7z (inspection only)
if ! need clang || ! need lld-link || ! need llvm-rc || ! need makensis; then
  $SUDO apt-get update
  $SUDO apt-get install -y clang lld llvm nsis p7zip-full curl unzip
fi
need node || { echo "Node 22 is required (add it to PATH)"; exit 1; }
need cargo || { curl -sSf https://sh.rustup.rs | sh -s -- -y; export PATH="$HOME/.cargo/bin:$PATH"; }

# 2. Rust target and cargo-xwin
rustup target add "$TARGET"
need cargo-xwin || cargo install --locked cargo-xwin

mkdir -p "$OUT"
cd "$REPO"

# 3. sing-box core (our fork with XHTTP) and its rule-sets, for Windows
need go || { echo "Go is required to build sing-box" >&2; exit 1; }
SINGBOX_SRC="$SINGBOX_SRC" node tools/fetch-singbox.mjs --target x86_64-pc-windows-msvc
cp src-tauri/resources/sing-box/sing-box.exe "$OUT/"

# 4. Xray for Windows, frontend dependencies, app + installer
node tools/fetch-xray.mjs --target "$TARGET"
npm ci
npx tauri build --runner cargo-xwin --target "$TARGET"

# 5. Collect artifacts
REL="src-tauri/target/$TARGET/release"
cp "$REL/quadra.exe" "$OUT/"
cp "$REL"/bundle/nsis/*-setup.exe "$OUT/"
echo; echo "Done. Artifacts in $OUT:"; ls -l "$OUT"
