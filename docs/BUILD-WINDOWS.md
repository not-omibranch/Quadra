# Building Quadra for Windows

Both ways produce `Quadra_<version>_x64-setup.exe` (NSIS, per-user install) and a bare `quadra.exe`.

## On the Windows PC (recommended for releases)

```powershell
git clone <quadra repo> ; git clone <sing-box-xhttp repo> ..\sing-box-xhttp   # sing-box fork sits next to the repo
cd quadra
powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1
```

The script installs missing prerequisites through winget (Node 22, Go, Git, Rustup, Visual Studio 2022
Build Tools with "Desktop development with C++"), builds `sing-box.exe` into `src-tauri\resources\sing-box\`,
fetches Xray, runs `npm ci` and `npm run tauri build`. The installer lands in
`src-tauri\target\release\bundle\nsis\`. If the Build Tools install fails, install them by hand
(tick "Desktop development with C++") and run the script again. WebView2 ships with Windows 10/11.

## Cross-compiling on Linux (Ubuntu 24.04, x86_64)

```bash
tools/build-windows-cross.sh
```

It installs clang, lld, llvm, NSIS, the `x86_64-pc-windows-msvc` Rust target and `cargo-xwin` (which downloads the
MSVC CRT and Windows SDK, `XWIN_ACCEPT_LICENSE=1` accepts the licence), then runs
`npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc`. Output goes to `out/windows/`.
Needs Node 22 and Go on PATH. Tauri marks cross builds experimental; the installer is unsigned.

Reference timing on a 96-core host: about 4 minutes cold (Windows SDK download included),
`quadra.exe` 4.5 MB, installer 15.9 MB (unpacks to about 67 MB, mostly Xray and geo data).
Cross-built binaries are not tested on Windows by the build; smoke-test the installer on a real PC before release.
