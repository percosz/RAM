<#
.SYNOPSIS
  Builds RAM (Tauri shell + Rust core) and a reproducible zip package.

.DESCRIPTION
  Runs the Rust tests, builds the React front-end with Vite, builds RAM.exe
  in release mode with the pinned toolchain and locked dependencies, and
  writes dist\RAM-<version>-x64.zip plus its SHA-256.

  The zip is reproducible: entries are stored in a fixed order with a fixed
  timestamp, the build remaps local paths, and the linker runs with /Brepro.
  The front-end bundle is deterministic for the same package-lock.json.

.PARAMETER TargetDir
  Cargo target directory. Defaults to a short path because the MSVC linker
  fails on very long ones.

.PARAMETER SkipTests
  Build and package without running the test suite.
#>
param(
    [string]$TargetDir = (Join-Path $env:USERPROFILE "wpt"),
    [switch]$SkipTests
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
if (Test-Path "C:\Program Files\nodejs") { $env:PATH = "C:\Program Files\nodejs;$env:PATH" }
$env:CARGO_TARGET_DIR = $TargetDir
# Strip machine-specific paths from the binary and make the linker output
# deterministic (/Brepro replaces the PE timestamp with a content hash).
$env:RUSTFLAGS = "--remap-path-prefix=$root=. --remap-path-prefix=$TargetDir=target --remap-path-prefix=$($env:USERPROFILE)=~ -C link-arg=/Brepro"

# Cargo and npm write progress to stderr, which the Stop preference would
# treat as a terminating error; run them with Continue and check exit codes.
if (-not $SkipTests) {
    & { $ErrorActionPreference = 'Continue'; cargo test --workspace --locked }
    if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}
Push-Location (Join-Path $root "app")
& { $ErrorActionPreference = 'Continue'; npm ci --no-fund --no-audit }
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "npm ci failed" }
& { $ErrorActionPreference = 'Continue'; npm run build }
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "front-end build failed" }
Pop-Location
& { $ErrorActionPreference = 'Continue'; cargo build --release --locked -p ram-tauri }
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$version = (Select-String -Path (Join-Path $root "app\src-tauri\Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$name = "RAM-$version-x64"
$dist = Join-Path $root "dist"
$stage = Join-Path $dist $name
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage, (Join-Path $stage "docs") | Out-Null

Copy-Item (Join-Path $TargetDir "release\RAM.exe") $stage
Copy-Item (Join-Path $root "LICENSE"), (Join-Path $root "NOTICE.md"), (Join-Path $root "README.md") $stage
Copy-Item (Join-Path $root "docs\*.md") (Join-Path $stage "docs")

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$fixedTime = [DateTimeOffset]::new(2020, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
$stream = [IO.File]::Open($zip, [IO.FileMode]::CreateNew)
$archive = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Create)
try {
    $files = Get-ChildItem $stage -Recurse -File | Sort-Object { $_.FullName.Substring($stage.Length + 1).Replace('\', '/') } -CaseSensitive
    foreach ($f in $files) {
        $entryName = "$name/" + $f.FullName.Substring($stage.Length + 1).Replace('\', '/')
        $entry = $archive.CreateEntry($entryName, [IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = $fixedTime
        $out = $entry.Open()
        try {
            $bytes = [IO.File]::ReadAllBytes($f.FullName)
            $out.Write($bytes, 0, $bytes.Length)
        } finally { $out.Dispose() }
    }
} finally {
    $archive.Dispose()
    $stream.Dispose()
}

$exeHash = (Get-FileHash (Join-Path $stage "RAM.exe") -Algorithm SHA256).Hash.ToLower()
$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$zipHash  $name.zip" | Set-Content -Encoding ascii (Join-Path $dist "$name.zip.sha256")

[pscustomobject]@{
    Package   = $zip
    ZipBytes  = (Get-Item $zip).Length
    ExeBytes  = (Get-Item (Join-Path $stage "RAM.exe")).Length
    ExeSha256 = $exeHash
    ZipSha256 = $zipHash
    Rustc     = (rustc -V)
} | Format-List
