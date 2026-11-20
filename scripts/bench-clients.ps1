<#
.SYNOPSIS
  Samples resource use of running Roblox clients, separate from the manager.

.DESCRIPTION
  Run this while one or several clients are in the same place with the same
  graphics settings, once with performance presets Off and once with a
  preset applied. It reports, per client process and for the system:

    CpuPercent     Share of the whole machine (all cores) over the window.
    WorkingSetMB / PrivateMB
    GpuPercent     From the "GPU Engine" performance counters, when Windows
                   exposes them for the process.
    Priority       Current priority class.

  Frame times are not measured here: Windows has no built-in per-process
  frame-time counter. Use PresentMon (https://github.com/GameTechDev/PresentMon)
  against the Main client's process ID for the same window, e.g.

    PresentMon.exe --process_id <pid> --timed 60 --output_file main.csv

  and compare the average and 99th-percentile of msBetweenPresents.

.EXAMPLE
  .\scripts\bench-clients.ps1 -Seconds 60 -Label preset-off
#>
param(
    [int]$Seconds = 60,
    [string]$Label = "clients",
    [string[]]$ImageNames = @("RobloxPlayerBeta")
)
$ErrorActionPreference = "Stop"
$cores = [Environment]::ProcessorCount
$procs = Get-Process -Name $ImageNames -ErrorAction SilentlyContinue
if (-not $procs) { "No client processes are running."; return }

$start = @{}
foreach ($p in $procs) { $start[$p.Id] = $p.TotalProcessorTime.TotalSeconds }

$gpu = @{}
$gpuSamples = 0
$counterAvailable = $true
$deadline = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $deadline) {
    if ($counterAvailable) {
        try {
            $samples = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples
            $gpuSamples++
            foreach ($s in $samples) {
                if ($s.InstanceName -match '^pid_(\d+)_') {
                    $id = [int]$Matches[1]
                    if ($start.ContainsKey($id)) { $gpu[$id] = ($gpu[$id] + $s.CookedValue) }
                }
            }
        } catch { $counterAvailable = $false }
    }
    Start-Sleep -Seconds 2
}

$rows = foreach ($p in $procs) {
    $now = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
    if (-not $now) { continue }
    [pscustomobject]@{
        Pid          = $p.Id
        Priority     = $now.PriorityClass
        CpuPercent   = [math]::Round(($now.TotalProcessorTime.TotalSeconds - $start[$p.Id]) / $Seconds / $cores * 100, 2)
        WorkingSetMB = [math]::Round($now.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($now.PrivateMemorySize64 / 1MB, 1)
        GpuPercent   = if ($counterAvailable -and $gpuSamples -gt 0) { [math]::Round(($gpu[$p.Id] / $gpuSamples), 1) } else { $null }
    }
}
$os = Get-CimInstance Win32_OperatingSystem
$summary = [pscustomobject]@{
    Label            = $Label
    When             = (Get-Date).ToString("s")
    Seconds          = $Seconds
    Clients          = @($rows).Count
    TotalCpuPercent  = [math]::Round((@($rows) | Measure-Object CpuPercent -Sum).Sum, 2)
    TotalWorkingSetMB = [math]::Round((@($rows) | Measure-Object WorkingSetMB -Sum).Sum, 1)
    SystemMemoryUsedPercent = [math]::Round((1 - $os.FreePhysicalMemory / $os.TotalVisibleMemorySize) * 100, 1)
    PerClient        = $rows
}
$rows | Format-Table -AutoSize
$summary | Select-Object Label, Seconds, Clients, TotalCpuPercent, TotalWorkingSetMB, SystemMemoryUsedPercent | Format-List
$summary | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 (Join-Path (Get-Location) "bench-$Label.json")
