# Benchmarks

> **Update (cookie-vault build).** The headline numbers below were first taken
> for the pre-vault build. For the current `RAM 0.1.0` with the encrypted
> vault, direct launch and multi-instance, measured on the same PC:
>
> | Metric | Value |
> |---|---|
> | Package (zip) | 0.57 MB (581,522 bytes) |
> | `RAM.exe` | 0.94 MB (986,624 bytes) |
> | Startup to first frame | 176-256 ms, median 205 ms (7 runs) |
> | Idle CPU | 0.00% of one core over 30 s |
> | Working set (idle) | 17.7 MB, 6 threads, one process |
> | Reproducible | Two clean builds in different directories produced identical exe and zip SHA-256 |
>
> The size and startup grew modestly (crypto crates, the embedded icon, and
> creating the multi-instance object at startup). The table below gives the
> fuller pre-vault measurements and the method, which is unchanged.

Measured on 2026-11-20 against the packaged release
(`RAM-0.1.0-x64.zip`, built by `scripts\package.ps1`).

**Test machine:** AMD Ryzen 5 5600 (6 cores, 12 threads), 32 GB RAM,
Windows 11 build 26300, 100% display scaling.

## What was and was not measured

| Measurement | RAM 0.1.0 | Legacy 3.7.2 |
|---|---|---|
| Manager startup, idle CPU, memory, package size, responsiveness with 100 accounts | **Measured** (below) | **Not measured** |
| Roblox client resource use and Main-client frame times, one and several clients | **Not measured** | **Not measured** |

Two gaps, stated plainly:

1. **No "before" numbers for the legacy app.** The legacy build was not run
   on the test machine: it is a third-party binary that handles Roblox
   credentials, downloads further components on first start and updates
   itself, and it was not run unattended. The only legacy figure below is
   the download size published on the release page. The same script that
   produced the RAM numbers measures any executable, so the comparison
   can be completed on your PC in a few minutes (see
   [Completing the comparison](#completing-the-comparison)).
2. **No client measurements.** No Roblox account was signed in and no
   client was run during development, so there are no frame-time or
   multi-client figures. The procedure is in
   [Client measurements](#client-measurements).

Because there is no measured baseline, no speed-up factor is claimed. The
10× goal is neither confirmed nor refuted by these numbers.

## RAM 0.1.0, manager only

All figures are for the manager's own process tree, with 100 saved accounts
and no Roblox client running.

| Metric | Result | How |
|---|---|---|
| Package size (zip) | **0.45 MB** (475 512 bytes) | `scripts\bench.ps1` |
| Installed size | 0.93 MB; `RAM.exe` is 0.76 MB (800 256 bytes) | Unpacked folder including documentation |
| Processes | **1** (3 threads, about 200 handles) | No helper processes, no embedded browser |
| Startup to first painted frame | **138–145 ms** (7 runs, median 139 ms) | `--bench-startup`: process entry to the end of the first paint |
| Startup as seen from outside | median **158 ms**, best 143 ms | `scripts\bench.ps1`: launch until the main window exists and its input queue is idle |
| First start after the build | 342 ms | Same script, first of 7 runs. A true cold start after a reboot was not measured. |
| Idle CPU | **0.00%** of one core in three 30 s windows; one earlier 60 s window read 0.42%, another 0.03% | Process CPU time; the clock resolution is about 16 ms, so one stray tick in 30 s reads as 0.05% |
| Working set | **17.5 MB** | |
| Private memory | **2.7 MB** | |
| Data file | 60 KB for 100 accounts | `store.json` |

When no client is running and nothing is queued, the manager runs no
timers: it only wakes for window messages.

### Where startup time goes

| Phase | Elapsed |
|---|---|
| Data file loaded | 1 ms |
| Main window created | 4 ms |
| Accounts view built | 33 ms |
| Theme applied and laid out | 51 ms |
| Running clients scanned | 63 ms |
| Table filled | 75 ms |
| First frame painted | 118–145 ms |

The other three views are built the first time they are opened. Before that
change, startup to first paint was about 245 ms.

### Responsiveness with 100 accounts

From `--bench-ui`, which drives the real window and includes painting.

| Operation | Time |
|---|---|
| Build the search index | 0.11 ms |
| Search keystroke: query, sort, refill and repaint the table | 4.0–11.3 ms depending on the number of matches |
| Move the selection one row and repaint | 2.0 ms |
| Same, including a full refresh of the details pane | 19.9 ms |
| Repaint the whole table | 6.5 ms |

The details pane is refreshed once after the selection settles (15 ms
later), so holding an arrow key scrolls at the 2 ms rate.

For scale, the same benchmark with **10 000** accounts: search keystroke
6.8–19.0 ms, selection 2.5 ms, table repaint 9.9 ms. The table is virtual,
so its cost depends on the visible rows, not the total.

### Sampling cost with clients

Resource sampling is off unless something needs it. With the Performance
view open it samples every 2 s (configurable); otherwise every 15 s, and
only when idle reminders or memory thresholds are enabled. GPU counters are
collected only while the Performance view is visible. The cost per sample
with real clients has not been measured.

## Legacy 3.7.2, what is known without running it

| Metric | Value | Source |
|---|---|---|
| Download size | 4.17 MB (4 372 924 bytes) | Release asset `Roblox.Account.Manager.3.7.2.zip` on GitHub |
| Runtime | .NET Framework WinForms application with an embedded Chromium browser (CefSharp) and Puppeteer for sign-in | Source audit, [LEGACY_AUDIT.md](LEGACY_AUDIT.md) |

Everything else (startup, idle CPU, memory of the process tree, size after
first-run downloads) needs a run on your machine.

## Completing the comparison

Run both on the same PC, after a reboot for a cold start, with nothing else
busy.

```powershell
.\scripts\bench.ps1 -Exe 'C:\path\to\Roblox Account Manager.exe' -PackagePath 'C:\path\to\RAM folder' -Label legacy-3.7.2
```

```powershell
.\scripts\bench.ps1 -Exe .\dist\RAM-0.1.0-x64\RAM.exe -PackagePath .\dist\RAM-0.1.0-x64.zip -Label RAM-0.1.0
```

Each run writes `bench-<label>.json` with startup (first, median, best),
idle CPU, working set, private memory, process count and package size for
the whole process tree, so helper and browser processes started by the
legacy app are included. Use the legacy folder as it is after first use, so
its downloaded components count toward package size. Both apps should hold
the same number of accounts.

## Client measurements

These need signed-in accounts and are yours to run. Keep the Roblox
version, graphics settings, place and server the same across runs.

1. **One client, presets Off.** Join the destination with Main. Record 60 s:

   ```powershell
   .\scripts\bench-clients.ps1 -Seconds 60 -Label one-client
   ```

   and, for frame times, PresentMon against Main's process ID (shown in the
   Performance view):

   ```powershell
   PresentMon.exe --process_id <pid> --timed 60 --output_file main-one-client.csv
   ```

2. **Several clients, presets Off** — only if Roblox lets more than one
   client run on your setup; RAM does not force it. Repeat both
   commands with `-Label n-clients-off`.
3. **Several clients, Balanced, then Low Resource.** Change the preset in
   the Performance view, wait 30 s, repeat with a new label.
4. Compare Main's average and 99th-percentile `msBetweenPresents` across the
   runs, and total client CPU and memory from the JSON files. The manager's
   own share is the `bench.ps1` result, which keeps its overhead separate
   from Roblox's.

If Windows refuses a priority or CPU-limit change for a client, the
Performance view says so in that client's Status column; record that too,
since it determines which controls are usable on your setup.

## Reproducible package

Two clean builds of the same commit in different build directories produced
byte-identical `RAM.exe` and zip files. The hashes for a build are
printed by `scripts\package.ps1` and written next to the zip.
