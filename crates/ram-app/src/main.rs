//! Waypoint: a Windows account manager for Roblox that hands sign-in and
//! launching to the user's browser and stores no credentials.
#![windows_subsystem = "windows"]

mod accounts;
mod app;
mod destinations;
mod dialogs;
mod performance;
mod settings;
mod theme;
mod ui;

use app::*;
use ram_core::clients::Clients;
use ram_core::model::{Account, Store};
use ram_core::perf::{IdleTracker, NotifyGate};
use ram_core::redact::Log;
use ram_core::search::SearchIndex;
use ram_core::store::{LoadOrigin, StoreError, StoreFile};
use ram_win::{ExitWatcher, WinBrowsers, WinJobs, WinProcesses, WinWindows, WindowShowHook};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use ui::Metrics;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const MAIN_CLASS: PCWSTR = w!("WaypointMain");
pub const PANEL_CLASS: PCWSTR = w!("WaypointPanel");

/// Keyboard shortcuts. Command IDs reuse the control IDs they trigger.
const ACCELERATORS: &[(u8, u16, u16)] = &[
    // (modifier flags, virtual key, command)
    (FCONTROL.0, b'1' as u16, 101),
    (FCONTROL.0, b'2' as u16, 102),
    (FCONTROL.0, b'3' as u16, 103),
    (FCONTROL.0, b'4' as u16, 104),
    (FCONTROL.0, b'F' as u16, accounts::CMD_FOCUS_SEARCH),
    (FCONTROL.0, b'N' as u16, accounts::ID_ADD),
    (FCONTROL.0, b'I' as u16, accounts::ID_IMPORT),
    (FCONTROL.0, b'L' as u16, accounts::ID_LAUNCH),
    (FCONTROL.0, b'A' as u16, accounts::CMD_SELECT_ALL),
    (FCONTROL.0, b'M' as u16, accounts::CMD_TOGGLE_MAIN),
    (FCONTROL.0, b'D' as u16, accounts::CMD_TOGGLE_FAVORITE),
    (FCONTROL.0, b'R' as u16, accounts::ID_REJOIN),
    (0, VK_F5.0, accounts::ID_CHECK),
    (0, VK_F6.0, accounts::ID_FOCUS_CLIENT),
    (FCONTROL.0 | FSHIFT.0, b'R' as u16, performance::ID_RESET),
];

struct Args {
    data_dir: Option<PathBuf>,
    seed: Option<usize>,
    bench_startup: bool,
    bench_ui: bool,
}

fn parse_args() -> Args {
    let mut a = Args { data_dir: None, seed: None, bench_startup: false, bench_ui: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data-dir" => a.data_dir = it.next().map(PathBuf::from),
            "--seed-demo" => a.seed = it.next().and_then(|n| n.parse().ok()),
            "--bench-startup" => a.bench_startup = true,
            "--bench-ui" => a.bench_ui = true,
            _ => {}
        }
    }
    a
}

fn main() {
    let started = Instant::now();
    let mut marks: Vec<(&str, f64)> = Vec::new();
    let mark = |name: &'static str, marks: &mut Vec<(&str, f64)>| marks.push((name, started.elapsed().as_secs_f64() * 1000.0));
    let args = parse_args();
    let custom_dir = args.data_dir.is_some();
    let data_dir = args
        .data_dir
        .clone()
        .or_else(|| ram_win::paths::local_app_data().map(|p| p.join("Waypoint")))
        .unwrap_or_else(|| PathBuf::from("."));

    // One instance per data directory; a second start brings the first to
    // the front.
    if !custom_dir {
        unsafe {
            let _mutex = CreateMutexW(None, true, w!("Local\\Waypoint.AccountManager"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                if let Ok(existing) = FindWindowW(MAIN_CLASS, None) {
                    let _ = ShowWindow(existing, SW_RESTORE);
                    let _ = SetForegroundWindow(existing);
                }
                return;
            }
            std::mem::forget(_mutex);
        }
    }

    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
    }

    let _ = std::fs::create_dir_all(&data_dir);
    let store_file = StoreFile::new(data_dir.join("store.json"));
    let log = Log::new(data_dir.join("waypoint.log"));
    let mut startup_note = String::new();
    let mut read_only = false;
    let mut store = match store_file.load() {
        Ok(loaded) => {
            if let LoadOrigin::Backup { preserved_as } = &loaded.origin {
                startup_note = format!(
                    "The data file could not be read, so the last backup was loaded. The unreadable file was kept as {}.",
                    preserved_as.display()
                );
            }
            loaded.store
        }
        Err(e) => {
            // Never overwrite a file this build cannot read.
            read_only = matches!(e, StoreError::NewerVersion { .. });
            startup_note = if read_only {
                format!("{e}. Changes in this session will not be saved.")
            } else {
                format!("{e}. Starting with an empty list.")
            };
            log.line(now(), &format!("store load: {e}"));
            Store::default()
        }
    };
    mark("store loaded", &mut marks);
    if let Some(n) = args.seed {
        seed_demo(&mut store, n);
    }

    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(main_proc),
            hInstance: ui::hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
            lpszClassName: MAIN_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        let panel = WNDCLASSEXW { lpfnWndProc: Some(panel_proc), lpszClassName: PANEL_CLASS, hIcon: HICON::default(), ..class };
        RegisterClassExW(&panel);
        dialogs::register();

        let rect = store.settings.window;
        let (x, y, cw, ch) = match rect {
            Some(r) if r.width() > 300 && r.height() > 200 => (r.left, r.top, r.width(), r.height()),
            _ => (CW_USEDEFAULT, CW_USEDEFAULT, 1180, 720),
        };
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            MAIN_CLASS,
            w!("Waypoint"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            cw,
            ch,
            None,
            None,
            Some(ui::hinstance()),
            None,
        )
        .expect("main window");

        mark("window created", &mut marks);
        let palette = theme::palette(&store.settings);
        let m = Metrics::new(ui::dpi_of(hwnd), store.settings.compact);
        let mut sysinfo = windows::Win32::System::SystemInformation::SYSTEM_INFO::default();
        windows::Win32::System::SystemInformation::GetSystemInfo(&mut sysinfo);

        let mut state = Box::new(App {
            hwnd,
            data_dir,
            store,
            store_file,
            read_only,
            log,
            clients: Clients::new(),
            index: SearchIndex::default(),
            procs: WinProcesses::new(),
            jobs: WinJobs::new(),
            windows: WinWindows::new(hwnd.0 as isize),
            browsers: WinBrowsers::new(),
            exit_watch: ExitWatcher::new(),
            show_hook: None,
            tray: None,
            brushes: theme::Brushes::new(&palette),
            palette,
            m,
            view: ViewId::Accounts,
            nav: [HWND::default(); 4],
            status: HWND::default(),
            panels: [HWND::default(); 4],
            accounts: Default::default(),
            destinations: Default::default(),
            performance: Default::default(),
            settings: Default::default(),
            samples: HashMap::new(),
            perf_notes: HashMap::new(),
            idle: IdleTracker::default(),
            gate: NotifyGate::default(),
            checking: Vec::new(),
            auth_cancel: None,
            cpus: sysinfo.dwNumberOfProcessors.max(1),
            created: [false; 4],
            links_restored: false,
            unwatched: false,
        });

        for (i, (_, title)) in VIEWS.iter().enumerate() {
            state.nav[i] = ui::button(hwnd, ID_NAV_FIRST + i as u16, title);
            state.panels[i] = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                PANEL_CLASS,
                None,
                WS_CHILD | WS_CLIPCHILDREN,
                0,
                0,
                10,
                10,
                Some(hwnd),
                None,
                Some(ui::hinstance()),
                None,
            )
            .unwrap_or_default();
        }
        state.status = ui::label(hwnd, ID_STATUS, "");
        install(state);
        let app = try_app().expect("installed");

        mark("state built", &mut marks);
        accounts::create(app);
        app.created[0] = true;
        mark("views created", &mut marks);
        set_fonts(hwnd, app.m.font);
        app.rebuild_index();
        app.apply_theme();
        mark("theme applied", &mut marks);

        // Adopt clients that are already running, then put back anything a
        // previous run changed and no longer should.
        app.rescan();
        app.apply_perf();
        mark("clients scanned", &mut marks);
        match WindowShowHook::install(hwnd.0 as isize, WM_PROC_SHOW, &[ram_win::process::DESKTOP_IMAGE, ram_win::process::STORE_IMAGE]) {
            Ok(hook) => app.show_hook = Some(hook),
            Err(e) => app.log(&format!("window hook unavailable: {e}")),
        }

        mark("hook installed", &mut marks);
        app.switch_view(ViewId::Accounts);
        mark("first view filled", &mut marks);
        if !startup_note.is_empty() {
            app.set_status(&startup_note);
        } else if app.store.accounts.is_empty() {
            app.set_status("Add an account with Ctrl+N, or import from Roblox Account Manager with Ctrl+I.");
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        if args.bench_startup {
            // Time from process entry to the first painted frame.
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let _ = std::fs::write(app.data_dir.join("bench-startup.txt"), format!("{ms:.1}"));
            let detail: String = marks.iter().map(|(n, t)| format!("{t:8.1} ms  {n}\n")).collect();
            let _ = std::fs::write(app.data_dir.join("bench-startup-phases.txt"), format!("{detail}{ms:8.1} ms  first frame painted\n"));
            post_quit_after_paint(hwnd);
        }
        if args.bench_ui {
            let report = accounts::bench(app);
            let _ = std::fs::write(app.data_dir.join("bench-ui.txt"), report);
            post_quit_after_paint(hwnd);
        }

        let accel_entries: Vec<ACCEL> = ACCELERATORS
            .iter()
            .map(|(mods, key, cmd)| ACCEL { fVirt: ACCEL_VIRT_FLAGS(*mods) | FVIRTKEY, key: *key, cmd: *cmd })
            .collect();
        let accel = CreateAcceleratorTableW(&accel_entries).unwrap_or_default();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Dialogs run their own loop; this one serves the main window.
            if TranslateAcceleratorW(hwnd, accel, &msg) != 0 {
                continue;
            }
            if accounts::pre_translate(&msg) {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyAcceleratorTable(accel);
    }
    uninstall();
}

fn post_quit_after_paint(hwnd: HWND) {
    ui::post(hwnd, WM_CLOSE, 0, 0);
}

/// Fills the store with demo accounts for benchmarks and UI checks.
fn seed_demo(store: &mut Store, n: usize) {
    let groups = ["Mains", "Farm", "Storage", "Trade", ""];
    let tags = ["gaia", "khei", "ultra", "silver", "alt"];
    while store.accounts.len() < n {
        let i = store.accounts.len();
        let id = store.alloc_id();
        store.accounts.push(Account {
            id,
            username: format!("demo_account_{i:03}"),
            alias: if i % 3 == 0 { format!("Alias {i}") } else { String::new() },
            group: groups[i % groups.len()].to_string(),
            tags: vec![tags[i % tags.len()].to_string(), tags[(i / 2) % tags.len()].to_string()],
            notes: format!("Demo note for account {i}.\nSecond line."),
            favorite: i % 7 == 0,
            created: now(),
            ..Default::default()
        });
    }
    if store.main_account.is_none() {
        store.main_account = store.accounts.first().map(|a| a.id);
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(app) = try_app() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_SIZE => {
            if wparam.0 as u32 != SIZE_MINIMIZED {
                app.layout();
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            let chrome = app.m.row() * 2 + app.m.gap() * 3 + app.m.px(46);
            info.ptMinTrackSize = POINT { x: app.m.px(900), y: (accounts::min_height(&app.m) + chrome).max(app.m.px(520)) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rc = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
            app.apply_theme();
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
            // Follows the system light/dark switch and high-contrast mode.
            let next = theme::palette(&app.store.settings);
            if next != app.palette {
                app.apply_theme();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ERASEBKGND => {
            let rc = ui::client_rect(hwnd);
            FillRect(HDC(wparam.0 as *mut _), &rc, app.brushes.window);
            LRESULT(1)
        }
        WM_COMMAND => {
            app.on_command(ui::loword(wparam.0), ui::hiword(wparam.0), HWND(lparam.0 as *mut _));
            LRESULT(0)
        }
        WM_NOTIFY => {
            let hdr = &*(lparam.0 as *const NMHDR);
            app.on_notify(hdr, lparam).unwrap_or(LRESULT(0))
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_SAVE => app.save_now(),
                TIMER_QUEUE => {
                    app.pump_queue();
                    // Launch timeouts are detected by observing.
                    if !app.clients.in_flight().is_empty() {
                        app.rescan();
                    }
                }
                TIMER_RESCAN => app.rescan(),
                TIMER_SAMPLE => app.sample(),
                TIMER_DETAILS => {
                    let _ = KillTimer(Some(hwnd), TIMER_DETAILS);
                    accounts::refresh_details(app);
                }
                TIMER_SEARCH => {
                    let _ = KillTimer(Some(hwnd), TIMER_SEARCH);
                    accounts::refresh(app);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PROC_EXIT | WM_PROC_SHOW => {
            app.rescan();
            LRESULT(0)
        }
        WM_CHECK_DONE => {
            let done = Box::from_raw(lparam.0 as *mut CheckDone);
            app.on_check_done(*done);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            // Returning to the manager is a natural moment to look again.
            if ui::loword(wparam.0) != 0 && !app.clients.in_flight().is_empty() {
                app.rescan();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if GetWindowPlacement(hwnd, &mut wp).is_ok() {
                let r = wp.rcNormalPosition;
                app.store.settings.window =
                    Some(ram_core::platform::Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
            }
            app.shutdown();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => match app.theme_message(msg, wparam, lparam) {
            Some(r) => r,
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        },
    }
}

/// Window procedure of the view panels: forwards control messages to the
/// application and paints the themed background.
pub unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(app) = try_app() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_ERASEBKGND => {
            let rc = ui::client_rect(hwnd);
            FillRect(HDC(wparam.0 as *mut _), &rc, app.brushes.window);
            LRESULT(1)
        }
        WM_COMMAND => {
            app.on_command(ui::loword(wparam.0), ui::hiword(wparam.0), HWND(lparam.0 as *mut _));
            LRESULT(0)
        }
        WM_NOTIFY => {
            let hdr = &*(lparam.0 as *const NMHDR);
            app.on_notify(hdr, lparam).unwrap_or(LRESULT(0))
        }
        WM_CONTEXTMENU => {
            // Right-click, Shift+F10 or the menu key on the account table.
            if HWND(wparam.0 as *mut _) == app.accounts.list {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                accounts::context_menu(app, x, y);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => match app.theme_message(msg, wparam, lparam) {
            Some(r) => r,
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        },
    }
}
