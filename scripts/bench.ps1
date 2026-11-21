<#
.SYNOPSIS
  Measures a desktop app's own overhead: startup, idle CPU, memory, size.

.DESCRIPTION
  Works for any GUI executable, so the same script measures the legacy
  Roblox Account Manager (before) and Waypoint (after) on the same PC.

  Reported values:
    StartupMs        Launch until the main window exists and its message
                     queue is idle (median of -Runs runs; the first run after
                     a reboot is the cold start).
    IdleCpuPercent   CPU used by the whole process tree while untouched for
                     -IdleSeconds, as a percentage of one core.
    WorkingSetMB     Sum of working sets over the process tree.
    PrivateMB        Sum of private bytes over the process tree.
    Processes        Number of processes in the tree (helpers, browsers).
    PackageMB        Size on disk of -PackagePath (folder or file).

  Roblox clients are not part of the tree, so this is the manager's overhead
  only. Use scripts\bench-clients.ps1 for client resource use and frame times.

.EXAMPLE
  .\scripts\bench.ps1 -Exe dist\RAM-0.1.0-x64\RAM.exe -Arguments '--data-dir','C:\tmp\wp-bench','--seed-demo','100' -PackagePath dist\RAM-0.1.0-x64.zip

.EXAMPLE
  .\scripts\bench.ps1 -Exe 'C:\RAM\Roblox Account Manager.exe' -PackagePath 'C:\RAM' -Label legacy-3.7.2
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [string[]]$Arguments = @(),
    [string]$PackagePath = "",
    [string]$Label = "",
    [int]$Runs = 5,
    [int]$IdleSeconds = 60,
    [int]$SettleSeconds = 10
)
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path $Exe).Path
if (-not $Label) { $Label = [IO.Path]::GetFileNameWithoutExtension($Exe) }

function Get-Tree([int]$RootPid) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $ids = New-Object System.Collections.Generic.List[int]
    $ids.Add($RootPid)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($p in $all) {
            if ($p.ParentProcessId -eq $ids[$i] -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) }
        }
    }
    $ids
}

function Measure-Tree([int]$RootPid) {
    $cpu = 0.0; $ws = 0; $private = 0; $count = 0
    foreach ($id in Get-Tree $RootPid) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($p) {
            $count++
            $cpu += $p.TotalProcessorTime.TotalSeconds
            $ws += $p.WorkingSet64
            $private += $p.PrivateMemorySize64
        }
    }
    [pscustomobject]@{ Cpu = $cpu; WorkingSet = $ws; Private = $private; Count = $count }
}

function Start-App {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = ($Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
    $psi.WorkingDirectory = Split-Path -Parent $Exe
    $psi.UseShellExecute = $false
    [System.Diagnostics.Process]::Start($psi)
}

function Stop-App($proc) {
    if ($proc.HasExited) { return }
    $null = $proc.CloseMainWindow()
    if (-not $proc.WaitForExit(5000)) {
        foreach ($id in (Get-Tree $proc.Id | Sort-Object -Descending)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    }
}

$startups = @()
for ($run = 1; $run -le $Runs; $run++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-App
    $null = $proc.WaitForInputIdle(60000)
    while ($proc.MainWindowHandle -eq [IntPtr]::Zero -and $sw.ElapsedMilliseconds -lt 60000 -and -not $proc.HasExited) {
        Start-Sleep -Milliseconds 5
        $proc.Refresh()
    }
    $sw.Stop()
    $startups += $sw.Elapsed.TotalMilliseconds
    if ($run -lt $Runs) { Stop-App $proc; Start-Sleep -Milliseconds 800 }
}

# The last instance stays up for the idle measurement.
Start-Sleep -Seconds $SettleSeconds
$before = Measure-Tree $proc.Id
Start-Sleep -Seconds $IdleSeconds
$after = Measure-Tree $proc.Id
Stop-App $proc

$packageBytes = 0
if ($PackagePath -and (Test-Path $PackagePath)) {
    $item = Get-Item $PackagePath
    $packageBytes = if ($item.PSIsContainer) { (Get-ChildItem $item.FullName -Recurse -File | Measure-Object Length -Sum).Sum } else { $item.Length }
}
$sorted = $startups | Sort-Object
$result = [pscustomobject]@{
    Label           = $Label
    When            = (Get-Date).ToString("s")
    Machine         = "$((Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()), $([math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)) GB, Windows $([Environment]::OSVersion.Version)"
    Runs            = $Runs
    StartupFirstMs  = [math]::Round($startups[0], 1)
    StartupMedianMs = [math]::Round($sorted[[int][math]::Floor($sorted.Count / 2)], 1)
    StartupMinMs    = [math]::Round($sorted[0], 1)
    IdleSeconds     = $IdleSeconds
    IdleCpuPercent  = [math]::Round(($after.Cpu - $before.Cpu) / $IdleSeconds * 100, 3)
    WorkingSetMB    = [math]::Round($after.WorkingSet / 1MB, 1)
    PrivateMB       = [math]::Round($after.Private / 1MB, 1)
    Processes       = $after.Count
    PackageMB       = [math]::Round($packageBytes / 1MB, 2)
}
$result | Format-List
$result | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path (Get-Location) "bench-$Label.json")
