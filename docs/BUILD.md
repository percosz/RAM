# Building Waypoint

## Requirements

- Windows 10 version 1809 or later, x64.
- Rust, the version pinned in `rust-toolchain.toml` (rustup installs it
  automatically), with the `x86_64-pc-windows-msvc` target.
- Visual Studio 2022 Build Tools with the "Desktop development with C++"
  workload (MSVC linker and Windows SDK).

```powershell
winget install Rustlang.Rustup
```

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

## Build and run

The MSVC linker fails when the build directory path is very long. If the
repository is in a deep folder, point Cargo at a short target directory
first:

```powershell
$env:CARGO_TARGET_DIR = "$env:USERPROFILE\wpt"
```

```powershell
cargo build --release -p ram-app
```

```powershell
& "$env:CARGO_TARGET_DIR\release\waypoint.exe"
```

The program is a single executable. It needs no installer, no runtime, no
administrator rights and no background service.

## Tests

```powershell
cargo test --workspace
```

- `ram-core` tests are pure and run anywhere.
- `ram-win` tests start short-lived child processes (`ping`, a PowerShell
  loop) to exercise priority, efficiency mode, Job Object limits and exit
  notifications. Four tests are ignored by default because they need the
  network or show a window; run them with:

```powershell
cargo test -p ram-win -- --ignored
```

## Reproducible package

```powershell
.\scripts\package.ps1
```

This runs the tests, builds with `--locked`, and writes
`dist\Waypoint-<version>-x64.zip` and its `.sha256`. The script remaps local
paths out of the binary and writes zip entries in a fixed order with a fixed
timestamp. With the same commit, the pinned toolchain and the same Windows
SDK version, the executable and zip hashes are the same on every run; the
script prints both so two builds can be compared.

## Command-line options

| Option | Purpose |
|---|---|
| `--data-dir <folder>` | Use this folder instead of `%LOCALAPPDATA%\Waypoint`. Allows a second instance. |
| `--seed-demo <n>` | Fill the store with `n` demo accounts (for UI checks and benchmarks). |
| `--bench-startup` | Write the time to the first painted frame to `bench-startup.txt` in the data folder, then exit. |
| `--bench-ui` | Measure search, selection and repaint times, write `bench-ui.txt`, then exit. |

## Benchmarks

```powershell
.\scripts\bench.ps1 -Exe dist\Waypoint-0.1.0-x64\waypoint.exe -Arguments '--data-dir','C:\tmp\wp-bench','--seed-demo','100' -PackagePath dist\Waypoint-0.1.0-x64.zip
```

The same script measures any other GUI executable, which is how the legacy
app is compared on the same PC. See [BENCHMARKS.md](BENCHMARKS.md).

## Layout

| Path | Contents |
|---|---|
| `crates/ram-core` | Model, store, importer, link validation, sign-in rules, client tracking, performance policy, redaction. No Windows dependency. |
| `crates/ram-win` | Windows implementations of the platform traits. |
| `crates/ram-app` | The Win32 application. |
| `scripts/` | Packaging and benchmark scripts. |
| `docs/` | Audit, research notes, decisions, migration, support matrix, benchmarks. |
