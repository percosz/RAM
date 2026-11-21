# Building RAM

RAM is a Rust workspace (the core and the Windows layer) plus a Tauri 2
application whose front-end is React + TypeScript built with Vite.

## Requirements

- Windows 10 version 1809 or later, x64, with the **WebView2 runtime** (part
  of Windows 11 and current Windows 10; also installed by Microsoft Edge).
- Rust, the version pinned in `rust-toolchain.toml` (rustup installs it
  automatically), with the `x86_64-pc-windows-msvc` target.
- Visual Studio 2022 Build Tools with the "Desktop development with C++"
  workload (MSVC linker and Windows SDK, which provides `rc.exe`).
- Node.js 20 or later with npm.

```powershell
winget install Rustlang.Rustup
```

```powershell
winget install OpenJS.NodeJS.LTS
```

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

## One-shot package

```powershell
.\scripts\package.ps1
```

Runs the Rust tests, `npm ci` + `npm run build` for the front-end, builds
`RAM.exe` in release mode with `--locked`, and writes
`dist\RAM-<version>-x64.zip` with its `.sha256`. The script remaps local paths
out of the binary, links with `/Brepro`, and writes zip entries in a fixed
order with a fixed timestamp, so the same commit with the same toolchain
produces the same hashes on every machine. Pass `-SkipTests` to skip the test
run.

## Step by step

The MSVC linker fails on very long paths; point Cargo at a short target
directory if the checkout is deep:

```powershell
$env:CARGO_TARGET_DIR = "$env:USERPROFILE\wpt"
```

```powershell
cd app; npm ci; npm run build; cd ..
```

```powershell
cargo build --release -p ram-tauri
```

The result is `$env:CARGO_TARGET_DIR\release\RAM.exe`, a single executable
with the front-end embedded. Debug builds embed it too (the `custom-protocol`
feature is on by default), so `cargo build -p ram-tauri` always yields a
working app after `npm run build`.

## Development loop

```powershell
cd app; npm run tauri dev -- --no-default-features
```

Starts Vite with hot reload and a debug app pointed at it (turning the
default feature off is what makes the app load from the dev server).

## Tests

```powershell
cargo test --workspace
```

- `ram-core` tests are pure Rust (vault, importer/exporter, ticket flow,
  login-check classification, status rules).
- `ram-win` tests spawn short-lived child processes to exercise the Windows
  layer. Four tests are `#[ignore]` (network, interactive UI); run them with
  `cargo test -p ram-win -- --ignored`.

## Layout

| Path | Contents |
|---|---|
| `crates/ram-core` | Model, store, vault, importer/exporter, Roblox API flows, status rules. No Windows dependency. |
| `crates/ram-win` | Windows implementations: processes, jobs, windows, Hello, DPAPI, WinHTTP, multi-instance, protocol launch. |
| `app/src-tauri` | The Tauri shell: state, commands, the per-account browser window. Builds `RAM.exe`. |
| `app/src` | React front-end: views, dialogs, themes. |
| `crates/ram-app` | The earlier native Win32 shell, kept for reference; not built. |
| `scripts/` | Packaging and benchmark scripts. |
| `docs/` | Audit, research, decisions, migration, support matrix, benchmarks. |

## Data

`%LOCALAPPDATA%\RAM\` holds `store.json` (accounts and settings, with
`store.json.bak`), `vault.bin` (encrypted cookies), `profiles\<id>\` (each
account's browser profile; its session is cleared when the window closes) and
`RAM.log` (redacted). `RAM_DATA_DIR` overrides the folder.
