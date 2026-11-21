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
`dist\RAM-<version>-x64.zip` with its `.sha256` (the portable build; it
cannot update itself). The script remaps local paths
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

## Releases and the auto-updater

Tagging a version and pushing the tag runs `.github/workflows/release.yml`,
which builds the NSIS installer with `tauri-action`, signs it, uploads the
installer, its `.sig`, `latest.json` (what the updater reads) and the
portable zip to a GitHub release whose notes are the matching `## <version>`
section of `CHANGELOG.md`:

```powershell
git tag v0.3.0; git push origin v0.3.0
```

The tag must match `version` in `app/src-tauri/tauri.conf.json`,
`app/src-tauri/Cargo.toml` and `app/package.json`.

Updates are signed with a minisign key pair. The public half is
`plugins.updater.pubkey` in `tauri.conf.json`; the private half lives outside
the repository (`%USERPROFILE%\.tauri\ram.key` on the machine that generated
it) and in the repository secret `TAURI_SIGNING_PRIVATE_KEY`. Losing the
private key means a new key pair and a release that users must install by
hand once. Generate a new pair with:

```powershell
cd app; npx tauri signer generate -w $env:USERPROFILE\.tauri\ram.key
```

The repository the updater reads from is `GITHUB_REPO` in
`app/src-tauri/src/cmd_update.rs`.

## Tests

```powershell
cargo test --workspace
```

- `ram-core` tests are pure Rust (vault, importer/exporter, ticket flow,
  login-check classification, status rules, client settings files, client
  log parsing).
- `ram-win` tests spawn short-lived child processes to exercise the Windows
  layer. Some tests are `#[ignore]` (network, interactive UI, and
  `sysbrowser::brave_round_trip`, which opens a Chromium browser on a scratch
  profile); run them with `cargo test -p ram-win -- --ignored`.

## Layout

| Path | Contents |
|---|---|
| `crates/ram-core` | Model, store, vault, importer/exporter, Roblox API flows, status rules. No Windows dependency. |
| `crates/ram-win` | Windows implementations: processes, jobs, windows, Hello, DPAPI, WinHTTP, multi-instance, protocol launch, launcher detection, the system-browser DevTools client. |
| `app/src-tauri` | The Tauri shell: state, commands, the client watcher, Open Browser, the updater. Builds `RAM.exe`. |
| `app/src` | React front-end: views, dialogs, themes. |
| `crates/ram-app` | The earlier native Win32 shell, kept for reference; not built. |
| `scripts/` | Packaging and benchmark scripts. |
| `docs/` | Audit, research, decisions, migration, support matrix, benchmarks. |

## Data

`%LOCALAPPDATA%\RAM\` holds `store.json` (accounts and settings, with
`store.json.bak`), `vault.bin` (encrypted cookies), `browser-profiles\<id>\` (each
account's browser profile; its cookies are wiped when the window closes) and
`RAM.log` (dated, levelled, redacted). `RAM_DATA_DIR` overrides the folder.
