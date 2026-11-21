//! The Accounts view: search, the account table, the details pane and the
//! launch bar.
//!
//! The table is a virtual (owner-data) report list drawn entirely in
//! `NM_CUSTOMDRAW`: each row paints its own background (an accent wash and a
//! left bar when selected), a rounded avatar chip in the Name column, the
//! per-column tone colours and a muted key marker for accounts with a stored
//! cookie. A second, dense "Classic" column set and an inline quick-launch
//! toolbar are offered through `store.settings.layout`.

use crate::app::{self, now, tone_color, tone_for_client, tone_for_sign_in, App, CookieState, Tone, LABEL, LABEL_MUTED, TIMER_DETAILS, TIMER_SEARCH};
use crate::avatars;
use crate::dialogs::{self, CheckRow, Field};
use crate::ui;
use ram_core::clients::ClientState;
use ram_core::import::{self, LegacyDecryptor, PlanAction, PlanKind};
use ram_core::links;
use ram_core::model::{Account, AccountId, BrowserBinding, LayoutMode, PresetKind, Target};
use ram_core::perf;
use ram_core::platform::{BrowserApi, BrowserInstall, BrowserProfile, WindowApi};
use ram_core::search::{self, QueryContext, SortKey};
use ram_core::signin::{self, CheckOutcome, RecoveryKind};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_SEARCH: u16 = 1000;
pub const ID_LIST: u16 = 1001;
pub const ID_ADD: u16 = 1002;
pub const ID_IMPORT: u16 = 1003;
pub const ID_DEST: u16 = 1004;
pub const ID_LAUNCH: u16 = 1005;
pub const ID_CANCEL: u16 = 1006;
pub const ID_REMOVE: u16 = 1007;
const ID_ALIAS: u16 = 1010;
const ID_GROUP: u16 = 1011;
const ID_TAGS: u16 = 1012;
const ID_NOTES: u16 = 1013;
const ID_FAV: u16 = 1014;
const ID_MAIN: u16 = 1015;
const ID_BROWSER: u16 = 1016;
const ID_PROFILE: u16 = 1017;
const ID_PERF: u16 = 1018;
const ID_RECOVER: u16 = 1019;
pub const ID_CHECK: u16 = 1020;
const ID_AUTHORIZE: u16 = 1021;
pub const ID_FOCUS_CLIENT: u16 = 1022;
pub const ID_REJOIN: u16 = 1023;
const ID_NEEDS_VERIFY: u16 = 1024;
const ID_RESUME: u16 = 1025;
const ID_SIGNIN_STATE: u16 = 1030;
const ID_CLIENT_STATE: u16 = 1031;
const ID_TITLE: u16 = 1032;
// Cookie controls in the details pane.
const ID_COOKIE_SET: u16 = 1040;
const ID_COOKIE_REVEAL: u16 = 1041;
const ID_COOKIE_COPY: u16 = 1042;
const ID_COOKIE_REMOVE: u16 = 1043;
const ID_COOKIE_STATE: u16 = 1044;
/// Owner-painted counts and legend bar above the launch bar.
const ID_LEGEND: u16 = 1045;
// Classic layout's inline quick-launch toolbar and bottom bar.
const ID_PLACE: u16 = 1050;
const ID_JOB: u16 = 1051;
const ID_JOIN: u16 = 1052;
const ID_ACCOUNTS_FILE: u16 = 1053;
const ID_OPEN_LOG: u16 = 1054;
pub const CMD_FOCUS_SEARCH: u16 = 1090;
pub const CMD_SELECT_ALL: u16 = 1091;
pub const CMD_TOGGLE_MAIN: u16 = 1092;
pub const CMD_TOGGLE_FAVORITE: u16 = 1093;
const CMD_SIGN_IN: u16 = 1094;
const CMD_RECOVERY: u16 = 1095;
const CMD_DEAUTHORIZE: u16 = 1096;

/// The modern column set: an avatar-led, information-rich table.
const MODERN_COLUMNS: [(&str, i32); 8] = [
    ("", 48),
    ("Name", 150),
    ("Username", 140),
    ("Group", 84),
    ("Tags", 104),
    ("Sign-in", 132),
    ("Client", 96),
    ("Last used", 84),
];

/// The classic column set: a dense, alyae-style table.
const CLASSIC_COLUMNS: [(&str, i32); 6] = [
    ("ONLINE", 56),
    ("USERNAME", 150),
    ("ALIAS", 140),
    ("GROUP", 120),
    ("MODE", 130),
    ("DESCRIPTION", 220),
];

#[derive(Default)]
pub struct AccountsView {
    pub list: HWND,
    search: HWND,
    add: HWND,
    import: HWND,
    remove: HWND,
    dest_label: HWND,
    dest: HWND,
    launch: HWND,
    cancel: HWND,
    legend: HWND,
    title: HWND,
    subtitle: HWND,
    labels: Vec<HWND>,
    alias: HWND,
    group: HWND,
    tags: HWND,
    notes: HWND,
    fav: HWND,
    main: HWND,
    browser: HWND,
    profile: HWND,
    perf: HWND,
    signin_state: HWND,
    signin_detail: HWND,
    recover: HWND,
    check: HWND,
    authorize: HWND,
    needs_verify: HWND,
    cookie_state: HWND,
    cookie_set: HWND,
    cookie_reveal: HWND,
    cookie_copy: HWND,
    cookie_remove: HWND,
    client_state: HWND,
    client_detail: HWND,
    focus_client: HWND,
    rejoin: HWND,
    resume: HWND,
    // Classic-only controls.
    place_edit: HWND,
    job_edit: HWND,
    join: HWND,
    accounts_file: HWND,
    open_log: HWND,
    /// Account IDs in display order; the virtual list reads from this.
    pub rows: Vec<AccountId>,
    dest_targets: Vec<Target>,
    installs: Vec<BrowserInstall>,
    profiles: Vec<BrowserProfile>,
    sort: SortKey,
    descending: bool,
    /// Set while the view updates its own controls, so the change
    /// notifications those updates raise are ignored.
    loading: bool,
    shown: Option<AccountId>,
    /// What the details combo boxes were last filled from.
    combo_signature: String,
    /// Which column set the list currently holds, so columns are rebuilt
    /// only when the layout actually changes.
    col_mode: Option<LayoutMode>,
}

pub fn is_accent(id: u16) -> bool {
    id == ID_LAUNCH
}

pub fn static_tone(app: &App, id: u16) -> Option<Tone> {
    let account = app.accounts.shown.and_then(|id| app.store.account(id));
    match id {
        ID_SIGNIN_STATE => Some(account.map(|a| tone_for_sign_in(a.sign_in.state)).unwrap_or(Tone::Muted)),
        ID_CLIENT_STATE => Some(account.map(|a| tone_for_client(&app.clients.state(a.id))).unwrap_or(Tone::Muted)),
        ID_COOKIE_STATE => Some(match app.accounts.shown.map(|id| app.cookie_state(id)) {
            Some(CookieState::Stored) => Tone::Good,
            _ => Tone::Muted,
        }),
        _ => None,
    }
}

fn is_classic(app: &App) -> bool {
    app.store.settings.layout == LayoutMode::Classic
}

pub fn create(app: &mut App) {
    let p = app.panels[0];
    let v = &mut app.accounts;
    v.search = ui::edit(p, ID_SEARCH);
    ui::set_cue(v.search, "Search name, alias, tag, notes \u{2014} or tag:farm, group:alts, is:fav");
    v.add = ui::button(p, ID_ADD, "Add");
    v.import = ui::button(p, ID_IMPORT, "Import\u{2026}");
    v.remove = ui::button(p, ID_REMOVE, "Remove");
    v.list = ui::listview(p, ID_LIST, 0);
    crate::theme::subclass_list(v.list);
    v.dest_label = ui::label(p, LABEL, "Launch to");
    v.dest = ui::combo(p, ID_DEST);
    v.launch = ui::button(p, ID_LAUNCH, "Launch");
    v.cancel = ui::button(p, ID_CANCEL, "Cancel queue");
    v.legend = ui::label(p, ID_LEGEND, "");

    v.title = ui::label(p, ID_TITLE, "");
    v.subtitle = ui::label(p, LABEL_MUTED, "");
    for text in ["Alias", "Group", "Tags (comma-separated)", "Notes", "Browser", "Profile", "Performance", "Sign-in", "Client", "Cookie"] {
        v.labels.push(ui::label(p, LABEL_MUTED, text));
    }
    v.alias = ui::edit(p, ID_ALIAS);
    v.group = ui::combo_editable(p, ID_GROUP);
    v.tags = ui::edit(p, ID_TAGS);
    v.notes = ui::edit_multiline(p, ID_NOTES);
    v.fav = ui::checkbox(p, ID_FAV, "Favorite");
    v.main = ui::checkbox(p, ID_MAIN, "Main account");
    v.browser = ui::combo(p, ID_BROWSER);
    v.profile = ui::combo(p, ID_PROFILE);
    v.perf = ui::combo(p, ID_PERF);
    v.signin_state = ui::label(p, ID_SIGNIN_STATE, "");
    v.signin_detail = ui::paragraph(p, LABEL_MUTED, "");
    v.recover = ui::button(p, ID_RECOVER, "Sign in in browser");
    v.check = ui::button(p, ID_CHECK, "Check");
    v.authorize = ui::button(p, ID_AUTHORIZE, "Authorize");
    v.needs_verify = ui::button(p, ID_NEEDS_VERIFY, "Needs verification");
    v.cookie_state = ui::label(p, ID_COOKIE_STATE, "");
    v.cookie_set = ui::button(p, ID_COOKIE_SET, "Set\u{2026}");
    v.cookie_reveal = ui::button(p, ID_COOKIE_REVEAL, "Reveal");
    v.cookie_copy = ui::button(p, ID_COOKIE_COPY, "Copy");
    v.cookie_remove = ui::button(p, ID_COOKIE_REMOVE, "Remove");
    v.client_state = ui::label(p, ID_CLIENT_STATE, "");
    v.client_detail = ui::paragraph(p, LABEL_MUTED, "");
    v.focus_client = ui::button(p, ID_FOCUS_CLIENT, "Focus");
    v.rejoin = ui::button(p, ID_REJOIN, "Rejoin");
    v.resume = ui::button(p, ID_RESUME, "Resume");

    v.place_edit = ui::edit(p, ID_PLACE);
    ui::set_cue(v.place_edit, "Place ID");
    v.job_edit = ui::edit(p, ID_JOB);
    ui::set_cue(v.job_edit, "Job ID (optional)");
    v.join = ui::button(p, ID_JOIN, "Join");
    v.accounts_file = ui::button(p, ID_ACCOUNTS_FILE, "Accounts file");
    v.open_log = ui::button(p, ID_OPEN_LOG, "Open log");
    v.installs = app.browsers.installed();

    subclass_legend(app.accounts.legend);
    set_columns(app);
}

/// Rebuilds the list's columns for the current layout. Cheap, but only run
/// when the layout changes.
fn set_columns(app: &mut App) {
    let list = app.accounts.list;
    while ui::send(list, LVM_DELETECOLUMN, 0, 0) != 0 {}
    let cols: &[(&str, i32)] = if is_classic(app) { &CLASSIC_COLUMNS } else { &MODERN_COLUMNS };
    for (i, (title, w)) in cols.iter().enumerate() {
        ui::list_add_column(list, i as i32, title, app.m.px(*w));
    }
    app.accounts.col_mode = Some(app.store.settings.layout);
    app.accounts.sort = SortKey::Default;
    app.accounts.descending = false;
}

/// Shows the controls for the active layout and hides the rest.
fn set_visibility(app: &App, classic: bool) {
    let v = &app.accounts;
    let modern = [
        v.title, v.subtitle, v.alias, v.group, v.tags, v.notes, v.fav, v.main, v.browser, v.profile, v.perf,
        v.signin_state, v.signin_detail, v.recover, v.check, v.authorize, v.needs_verify, v.cookie_state,
        v.cookie_set, v.cookie_reveal, v.cookie_copy, v.cookie_remove, v.client_state, v.client_detail,
        v.focus_client, v.rejoin, v.resume,
    ];
    for h in modern {
        ui::show(h, !classic);
    }
    for h in &v.labels {
        ui::show(*h, !classic);
    }
    for h in [v.place_edit, v.job_edit, v.join, v.accounts_file, v.open_log] {
        ui::show(h, classic);
    }
}

pub fn layout(app: &mut App) {
    let classic = is_classic(app);
    set_visibility(app, classic);
    if classic {
        layout_classic(app);
    } else {
        layout_modern(app);
    }
}

fn layout_modern(app: &mut App) {
    let rc = ui::client_rect(app.panels[0]);
    let m = &app.m;
    let v = &app.accounts;
    let (gap, row) = (m.gap(), m.row());
    let detail_w = m.px(330);
    let left_w = rc.right - detail_w - 3 * gap;
    let bw = m.px(84);

    // Left: search row, table, legend bar, launch bar.
    let mut x = gap;
    ui::place(v.search, x, 0, left_w - 3 * (bw + gap), row);
    x += left_w - 3 * (bw + gap) + gap;
    for b in [v.add, v.import, v.remove] {
        ui::place(b, x, 0, bw, row);
        x += bw + gap;
    }
    let list_top = row + gap;
    let launch_y = rc.bottom - row - gap;
    let legend_y = launch_y - row - gap;
    ui::place(v.list, gap, list_top, left_w, legend_y - gap - list_top);
    ui::place(v.legend, gap, legend_y, left_w, row);
    // The Name column takes whatever width the fixed columns leave. Columns
    // adapt to the width: the least important ones give way first so the
    // table never needs a horizontal scroll bar.
    let mut widths: Vec<i32> = MODERN_COLUMNS.iter().map(|(_, w)| m.px(*w)).collect();
    let available = left_w - m.px(24);
    let name_min = m.px(120);
    for drop in [4usize, 7, 3, 2] {
        let fixed: i32 = widths.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, w)| *w).sum();
        if fixed + name_min <= available {
            break;
        }
        widths[drop] = 0;
    }
    let fixed: i32 = widths.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, w)| *w).sum();
    widths[1] = (available - fixed).max(name_min);
    for (i, w) in widths.iter().enumerate() {
        ui::send(v.list, LVM_SETCOLUMNWIDTH, i, *w as isize);
    }
    let label_w = m.px(64);
    ui::place(v.dest_label, gap, launch_y + m.px(5), label_w, row);
    let launch_w = m.px(96);
    let cancel_w = m.px(108);
    let combo_w = left_w - label_w - launch_w - cancel_w - 3 * gap;
    ui::place(v.dest, gap + label_w + gap, launch_y, combo_w, row * 14);
    ui::place(v.launch, gap + label_w + combo_w + 2 * gap, launch_y, launch_w, row);
    ui::place(v.cancel, gap + label_w + combo_w + launch_w + 3 * gap, launch_y, cancel_w, row);

    // Right: details pane.
    let dx = rc.right - detail_w - gap;
    let dw = detail_w;
    let lh = m.px(18);
    let mut y = 0;
    ui::place(v.title, dx, y, dw, row - m.px(4));
    y += row - m.px(4);
    ui::place(v.subtitle, dx, y, dw, lh);
    y += lh + gap;
    let half = (dw - gap) / 2;
    let field = |label: HWND, ctl: HWND, x: i32, w: i32, y: i32, h: i32| {
        ui::place(label, x, y, w, lh);
        ui::place(ctl, x, y + lh, w, h);
    };
    field(v.labels[0], v.alias, dx, half, y, row);
    field(v.labels[1], v.group, dx + half + gap, half, y, row * 12);
    y += lh + row + gap;
    field(v.labels[2], v.tags, dx, dw, y, row);
    y += lh + row + gap;

    // Fixed-height blocks below the notes; the notes box takes the rest.
    let notes_h = (rc.bottom - y - lh - below_notes(m) - gap).max(row * 2);
    field(v.labels[3], v.notes, dx, dw, y, notes_h);
    y += lh + notes_h + gap;
    ui::place(v.fav, dx, y, half, row);
    ui::place(v.main, dx + half + gap, y, half, row);
    y += row + gap;
    field(v.labels[4], v.browser, dx, half, y, row * 10);
    field(v.labels[5], v.profile, dx + half + gap, half, y, row * 12);
    y += lh + row + gap;
    field(v.labels[6], v.perf, dx, dw, y, row * 8);
    y += lh + row + gap;

    ui::place(v.labels[7], dx, y, m.px(60), lh);
    ui::place(v.signin_state, dx + m.px(60), y, dw - m.px(60), lh);
    y += lh;
    ui::place(v.signin_detail, dx, y, dw, lh * 2);
    y += lh * 2 + m.px(2);
    ui::place(v.recover, dx, y, half, row);
    ui::place(v.needs_verify, dx + half + gap, y, half, row);
    y += row + gap / 2;
    ui::place(v.check, dx, y, half, row);
    ui::place(v.authorize, dx + half + gap, y, half, row);
    y += row + gap;

    // Cookie row: a status label and the four vault actions.
    ui::place(v.labels[9], dx, y, m.px(60), lh);
    ui::place(v.cookie_state, dx + m.px(60), y, dw - m.px(60), lh);
    y += lh;
    let b4 = (dw - 3 * gap) / 4;
    ui::place(v.cookie_set, dx, y, b4, row);
    ui::place(v.cookie_reveal, dx + (b4 + gap), y, b4, row);
    ui::place(v.cookie_copy, dx + 2 * (b4 + gap), y, b4, row);
    ui::place(v.cookie_remove, dx + 3 * (b4 + gap), y, b4, row);
    y += row + gap;

    ui::place(v.labels[8], dx, y, m.px(60), lh);
    ui::place(v.client_state, dx + m.px(60), y, dw - m.px(60), lh);
    y += lh;
    ui::place(v.client_detail, dx, y, dw, lh * 2);
    y += lh * 2 + m.px(2);
    let b3 = (dw - 2 * gap) / 3;
    ui::place(v.focus_client, dx, y, b3, row);
    ui::place(v.rejoin, dx + b3 + gap, y, b3, row);
    ui::place(v.resume, dx + 2 * (b3 + gap), y, b3, row);
}

fn layout_classic(app: &mut App) {
    let rc = ui::client_rect(app.panels[0]);
    let m = &app.m;
    let v = &app.accounts;
    let (gap, row) = (m.gap(), m.row());
    let full_w = rc.right - 2 * gap;

    // Top toolbar: search, inline quick-launch, then Add/Import/Remove.
    let bw = m.px(78);
    let place_w = m.px(96);
    let job_w = m.px(96);
    let join_w = m.px(56);
    let right_block = place_w + job_w + join_w + 3 * bw + 6 * gap;
    let search_w = (full_w - right_block).max(m.px(160));
    let mut x = gap;
    ui::place(v.search, x, 0, search_w, row);
    x += search_w + gap;
    ui::place(v.place_edit, x, 0, place_w, row);
    x += place_w + gap;
    ui::place(v.job_edit, x, 0, job_w, row);
    x += job_w + gap;
    ui::place(v.join, x, 0, join_w, row);
    x += join_w + gap;
    for b in [v.add, v.import, v.remove] {
        ui::place(b, x, 0, bw, row);
        x += bw + gap;
    }

    let list_top = row + gap;
    let launch_y = rc.bottom - row - gap;
    let info_y = launch_y - row - gap;
    ui::place(v.list, gap, list_top, full_w, info_y - gap - list_top);

    // Dense proportional columns: a fixed ONLINE dot, GROUP and MODE, and
    // flexible USERNAME, ALIAS and DESCRIPTION.
    let online = m.px(56);
    let group = m.px(120);
    let mode = m.px(130);
    let avail = (full_w - m.px(24) - online - group - mode).max(m.px(270));
    let username = (avail * 3 / 10).max(m.px(90));
    let alias = (avail * 3 / 10).max(m.px(80));
    let desc = (avail - username - alias).max(m.px(100));
    for (i, w) in [online, username, alias, group, mode, desc].iter().enumerate() {
        ui::send(v.list, LVM_SETCOLUMNWIDTH, i, *w as isize);
    }

    // Info row: totals on the left, data-folder and log buttons on the right.
    let btn_w = m.px(110);
    ui::place(v.open_log, rc.right - gap - btn_w, info_y, btn_w, row);
    ui::place(v.accounts_file, rc.right - gap - 2 * btn_w - gap, info_y, btn_w, row);
    ui::place(v.legend, gap, info_y, full_w - 2 * (btn_w + gap), row);

    // Launch bar: the destination fallback, Launch and Cancel.
    let label_w = m.px(64);
    let launch_w = m.px(96);
    let cancel_w = m.px(108);
    let combo_w = full_w - label_w - launch_w - cancel_w - 3 * gap;
    ui::place(v.dest_label, gap, launch_y + m.px(5), label_w, row);
    ui::place(v.dest, gap + label_w + gap, launch_y, combo_w, row * 14);
    ui::place(v.launch, gap + label_w + combo_w + 2 * gap, launch_y, launch_w, row);
    ui::place(v.cancel, gap + label_w + combo_w + launch_w + 3 * gap, launch_y, cancel_w, row);
}

/// Height of the fixed blocks under the notes box.
fn below_notes(m: &ui::Metrics) -> i32 {
    let (gap, row, lh) = (m.gap(), m.row(), m.px(18));
    (row + gap)
        + (lh + row + gap) * 2
        + (lh * 3 + m.px(2) + row * 2 + gap / 2 + gap)
        + (lh + row + gap)
        + (lh * 3 + m.px(2) + row)
}

/// Smallest panel height at which the details pane fits without overlap.
pub fn min_height(m: &ui::Metrics) -> i32 {
    let (gap, row, lh) = (m.gap(), m.row(), m.px(18));
    (row - m.px(4)) + lh + gap + (lh + row + gap) * 2 + lh + row * 2 + gap + below_notes(m) + gap
}

// ---- data ------------------------------------------------------------------

fn query_context(app: &App) -> QueryContext {
    QueryContext { main: app.store.main_account, running: app.clients.running_accounts() }
}

/// Re-runs the search and sort, then repaints the table and details.
pub fn refresh(app: &mut App) {
    if app.accounts.col_mode != Some(app.store.settings.layout) {
        set_columns(app);
        layout(app);
    }
    let selected: std::collections::HashSet<AccountId> = selected_ids(app).into_iter().collect();
    let query = ui::get_text(app.accounts.search);
    let mut ids = app.index.query(&query, &query_context(app));
    search::sort_ids(&app.store, &mut ids, app.accounts.sort, app.accounts.descending);
    app.accounts.rows = ids;
    app.accounts.loading = true;
    ui::list_set_count(app.accounts.list, app.accounts.rows.len());
    ui::list_set_selected(app.accounts.list, None, false);
    let mut first = true;
    for (i, id) in app.accounts.rows.iter().enumerate() {
        if selected.contains(id) {
            if first {
                ui::list_focus_row(app.accounts.list, i);
                first = false;
            } else {
                ui::list_set_selected(app.accounts.list, Some(i), true);
            }
        }
    }
    app.accounts.loading = false;
    ui::invalidate(app.accounts.legend);
    refresh_destinations(app);
    refresh_details(app);
}

/// Repaints row text and the details pane without changing order or
/// selection. Cheap enough to call on every state change.
pub fn refresh_rows(app: &mut App) {
    ui::invalidate(app.accounts.list);
    ui::invalidate(app.accounts.legend);
    refresh_details(app);
}

fn refresh_destinations(app: &mut App) {
    let previous = ui::combo_selected(app.accounts.dest).and_then(|i| app.accounts.dest_targets.get(i).cloned());
    let mut targets = Vec::new();
    let mut names = Vec::new();
    for g in app.store.games.iter().filter(|g| g.pinned) {
        targets.push(Target::Place { place_id: g.place_id });
        names.push(g.name.clone());
    }
    let mut dests: Vec<_> = app.store.destinations.iter().collect();
    dests.sort_by_key(|d| d.name.to_lowercase());
    for d in dests {
        if targets.contains(&d.target) {
            continue;
        }
        names.push(format!("{} \u{2014} {}", d.name, d.target.durability().label()));
        targets.push(d.target.clone());
    }
    for g in app.store.games.iter().filter(|g| !g.pinned) {
        let t = Target::Place { place_id: g.place_id };
        if !targets.contains(&t) {
            names.push(g.name.clone());
            targets.push(t);
        }
    }
    let v = &mut app.accounts;
    v.loading = true;
    ui::combo_clear(v.dest);
    for n in &names {
        ui::combo_add(v.dest, n);
    }
    let default = app
        .store
        .settings
        .default_destination
        .and_then(|id| app.store.destinations.iter().find(|d| d.id == id))
        .map(|d| d.target.clone());
    let pick = previous.or(default).and_then(|t| targets.iter().position(|x| *x == t)).unwrap_or(0);
    ui::combo_select(v.dest, pick as i32);
    v.dest_targets = targets;
    v.loading = false;
}

pub fn selected_ids(app: &App) -> Vec<AccountId> {
    ui::list_selected(app.accounts.list).into_iter().filter_map(|i| app.accounts.rows.get(i).copied()).collect()
}

fn single(app: &App) -> Option<AccountId> {
    let ids = selected_ids(app);
    (ids.len() == 1).then(|| ids[0])
}

pub fn selected_target(app: &App) -> Option<Target> {
    ui::combo_selected(app.accounts.dest).and_then(|i| app.accounts.dest_targets.get(i).cloned())
}

fn client_text(app: &App, id: AccountId) -> String {
    match app.clients.state(id) {
        ClientState::Running => "Running".to_string(),
        ClientState::Exited { at } => format!("Exited {}", ui::ago(now(), Some(at))),
        other => other.label().to_string(),
    }
}

/// The classic MODE cell: the live client state when there is one, otherwise
/// the sign-in state.
fn mode_label(app: &App, a: &Account) -> String {
    match app.clients.state(a.id) {
        ClientState::Idle | ClientState::Exited { .. } => a.sign_in.state.label().to_string(),
        other => other.label().to_string(),
    }
}

fn mode_tone(app: &App, a: &Account) -> Tone {
    match app.clients.state(a.id) {
        ClientState::Idle | ClientState::Exited { .. } => tone_for_sign_in(a.sign_in.state),
        other => tone_for_client(&other),
    }
}

/// The text of one cell for the active layout. The virtual list and the
/// owner-draw path share this, so both stay in step.
fn cell(app: &App, a: &Account, column: i32) -> String {
    if is_classic(app) {
        cell_classic(app, a, column)
    } else {
        cell_modern(app, a, column)
    }
}

fn cell_modern(app: &App, a: &Account, column: i32) -> String {
    match column {
        0 => {
            if app.store.is_main(a.id) {
                "Main".into()
            } else if a.favorite {
                "\u{2605}".into()
            } else {
                String::new()
            }
        }
        1 => a.display_name().to_string(),
        2 => a.username.clone(),
        3 => import::group_display_name(&a.group).to_string(),
        4 => a.tags.join(", "),
        5 => {
            if app.checking.contains(&a.id) {
                "Checking\u{2026}".into()
            } else {
                a.sign_in.state.label().to_string()
            }
        }
        6 => client_text(app, a.id),
        7 => ui::ago(now(), a.last_used),
        _ => String::new(),
    }
}

fn cell_classic(app: &App, a: &Account, column: i32) -> String {
    match column {
        // The dot is drawn in the owner-draw path; the text stays empty.
        0 => String::new(),
        1 => a.username.clone(),
        2 => a.alias.clone(),
        3 => import::group_display_name(&a.group).to_string(),
        4 => mode_label(app, a),
        5 => a.notes.lines().next().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// The tone colour of a cell for the active layout.
fn cell_tone(app: &App, a: &Account, classic: bool, col: usize) -> Tone {
    if classic {
        match col {
            0 => {
                if matches!(app.clients.state(a.id), ClientState::Running) {
                    Tone::Good
                } else {
                    Tone::Muted
                }
            }
            1 => Tone::Text,
            4 => mode_tone(app, a),
            _ => Tone::Muted,
        }
    } else {
        match col {
            5 => tone_for_sign_in(a.sign_in.state),
            6 => tone_for_client(&app.clients.state(a.id)),
            0 if app.store.is_main(a.id) => Tone::Good,
            2 | 3 | 4 | 7 => Tone::Muted,
            _ => Tone::Text,
        }
    }
}

/// Updates the details pane for the current selection.
pub fn refresh_details(app: &mut App) {
    let ids = selected_ids(app);
    let one = if ids.len() == 1 { app.store.account(ids[0]).cloned() } else { None };
    let editable = one.is_some();
    app.accounts.loading = true;
    app.accounts.shown = one.as_ref().map(|a| a.id);
    let cookie = app.accounts.shown.map(|id| app.cookie_state(id)).unwrap_or(CookieState::Missing);
    let stored = cookie == CookieState::Stored;
    let v = &app.accounts;
    for h in [v.alias, v.group, v.tags, v.notes, v.fav, v.main, v.browser, v.profile, v.perf, v.recover, v.check, v.authorize, v.needs_verify] {
        ui::enable(h, editable);
    }
    ui::enable(v.remove, !ids.is_empty());
    ui::enable(v.launch, !ids.is_empty());
    ui::enable(v.cancel, app.clients.queued_len() > 0);
    ui::enable(v.cookie_set, editable);
    ui::enable(v.cookie_reveal, editable && stored);
    ui::enable(v.cookie_copy, editable && stored);
    ui::enable(v.cookie_remove, editable && stored);
    ui::set_text_if_changed(
        v.cookie_state,
        if !editable {
            ""
        } else {
            match cookie {
                CookieState::Stored => "Stored",
                CookieState::Missing => "None",
                CookieState::NoVault => "No vault",
            }
        },
    );

    match &one {
        None => {
            let title = match ids.len() {
                0 if app.store.accounts.is_empty() => "No accounts yet".to_string(),
                0 => "No account selected".to_string(),
                n => format!("{n} accounts selected"),
            };
            ui::set_text_if_changed(v.title, &title);
            let batch = search::batch_targets(&app.store, &ids);
            let sub = if ids.len() > 1 && batch.len() != ids.len() {
                "Main is excluded from batch launches.".to_string()
            } else if ids.is_empty() {
                "Select a row to see its details.".to_string()
            } else {
                String::new()
            };
            ui::set_text_if_changed(v.subtitle, &sub);
            for h in [v.alias, v.tags, v.notes, v.signin_state, v.signin_detail, v.client_state, v.client_detail] {
                ui::set_text_if_changed(h, "");
            }
            ui::set_text_if_changed(v.group, "");
            ui::set_checked(v.fav, false);
            ui::set_checked(v.main, false);
            for h in [v.browser, v.profile, v.perf] {
                ui::combo_clear(h);
            }
            for h in [v.focus_client, v.rejoin, v.resume] {
                ui::enable(h, false);
            }
        }
        Some(a) => {
            ui::set_text_if_changed(v.title, a.display_name());
            let id_text = match (a.roblox_user_id, a.user_id_confirmed) {
                (Some(id), true) => format!("{} \u{00B7} ID {id}", a.username),
                (Some(id), false) => format!("{} \u{00B7} ID {id} (unconfirmed)", a.username),
                (None, _) => a.username.clone(),
            };
            ui::set_text_if_changed(v.subtitle, &id_text);
            ui::set_text_if_changed(v.alias, &a.alias);
            ui::set_text_if_changed(v.tags, &a.tags.join(", "));
            ui::set_text_if_changed(v.notes, &a.notes.replace("\r\n", "\n").replace('\n', "\r\n"));
            if ui::focused() != v.group && unsafe { GetParent(ui::focused()).unwrap_or_default() } != v.group {
                ui::combo_clear(v.group);
                for g in app.store.groups() {
                    ui::combo_add(v.group, &g);
                }
                ui::set_text(v.group, &a.group);
            }
            ui::set_checked(v.fav, a.favorite);
            ui::set_checked(v.main, app.store.is_main(a.id));

            let signature = format!("{}|{:?}|{}", a.id, app.store.perf.preset, v.installs.len());
            let rebuild = signature != v.combo_signature;
            if rebuild {
                ui::combo_clear(v.browser);
                ui::combo_add(v.browser, "Default browser");
                for b in &v.installs {
                    ui::combo_add(v.browser, &b.display_name);
                }
                ui::combo_clear(v.perf);
                ui::combo_add(v.perf, &format!("Use global preset ({})", app.store.perf.preset.label()));
                for p in [PresetKind::Off, PresetKind::Balanced, PresetKind::LowResource] {
                    ui::combo_add(v.perf, p.label());
                }
            }
            let browser_index = a
                .browser
                .as_ref()
                .and_then(|b| v.installs.iter().position(|i| i.id == b.browser))
                .map(|i| i + 1)
                .unwrap_or(0);
            ui::combo_select(v.browser, browser_index as i32);

            ui::combo_select(
                v.perf,
                match a.perf_override {
                    None => 0,
                    Some(PresetKind::Off) => 1,
                    Some(PresetKind::Balanced) => 2,
                    Some(PresetKind::LowResource) => 3,
                },
            );

            let checking = app.checking.contains(&a.id);
            let state_text = if checking {
                "Checking\u{2026}".to_string()
            } else {
                match a.sign_in.last_check {
                    Some(t) => format!("{} \u{00B7} {}", a.sign_in.state.label(), ui::ago(now(), Some(t))),
                    None => a.sign_in.state.label().to_string(),
                }
            };
            ui::set_text_if_changed(v.signin_state, &state_text);
            let detail = if a.sign_in.detail.is_empty() {
                "Not checked yet. Sign in in the browser; the manager stores no credentials.".to_string()
            } else {
                a.sign_in.detail.clone()
            };
            ui::set_text_if_changed(v.signin_detail, &detail);
            ui::set_text_if_changed(v.recover, signin::recovery(a.sign_in.state).label);
            ui::enable(v.check, !checking);

            let state = app.clients.state(a.id);
            ui::set_text_if_changed(v.client_state, &client_text(app, a.id));
            let link = app.clients.link_for_account(a.id);
            let detail = match &state {
                ClientState::Idle => "No client observed for this account.".to_string(),
                ClientState::Queued => "Waiting for its turn in the launch queue.".to_string(),
                ClientState::Paused { reason } => reason.clone(),
                ClientState::Launching { .. } => "Handed to the browser. Waiting for a new client process.".to_string(),
                ClientState::Running => match link {
                    Some(l) => {
                        let exempt = perf::effective(&app.store, Some(a.id)).label();
                        format!("Process {} \u{2014} {}. Preset: {exempt}.", l.process.pid, l.evidence.label())
                    }
                    None => String::new(),
                },
                ClientState::Unconfirmed => {
                    "A client started but cannot be tied to this account. Assign it in Performance.".to_string()
                }
                ClientState::Exited { .. } => "The client process ended.".to_string(),
                ClientState::Failed { reason } => reason.clone(),
            };
            ui::set_text_if_changed(v.client_detail, &detail);
            ui::enable(v.focus_client, link.is_some());
            ui::enable(v.rejoin, app.clients.last_target(a.id).is_some() && link.is_none());
            ui::enable(v.resume, matches!(state, ClientState::Paused { .. }));
        }
    }
    for h in [app.accounts.signin_state, app.accounts.client_state, app.accounts.cookie_state] {
        ui::invalidate(h);
    }
    app.accounts.loading = false;
    match one {
        Some(a) => {
            let signature = format!("{}|{:?}|{}", a.id, app.store.perf.preset, app.accounts.installs.len());
            if signature != app.accounts.combo_signature {
                app.accounts.combo_signature = signature;
                refresh_profiles(app, &a);
            }
        }
        None => app.accounts.combo_signature.clear(),
    }
}

fn refresh_profiles(app: &mut App, a: &Account) {
    let browser_index = ui::combo_selected(app.accounts.browser).unwrap_or(0);
    app.accounts.loading = true;
    ui::combo_clear(app.accounts.profile);
    if browser_index == 0 {
        ui::combo_add(app.accounts.profile, "Choose in browser");
        ui::combo_select(app.accounts.profile, 0);
        ui::enable(app.accounts.profile, false);
        app.accounts.profiles.clear();
    } else {
        let id = app.accounts.installs[browser_index - 1].id.clone();
        let profiles = app.browsers.profiles(&id);
        for p in &profiles {
            ui::combo_add(app.accounts.profile, &p.display_name);
        }
        let current = a.browser.as_ref().filter(|b| b.browser == id).and_then(|b| profiles.iter().position(|p| p.key == b.profile));
        if let Some(i) = current {
            ui::combo_select(app.accounts.profile, i as i32);
        }
        ui::enable(app.accounts.profile, true);
        app.accounts.profiles = profiles;
    }
    app.accounts.loading = false;
}

// ---- owner draw ------------------------------------------------------------

/// Mixes two COLORREF values; `t` is the weight of `b`.
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, shift: u32| (c >> shift) & 0xff;
    let mix = |x: u32, y: u32| ((x as f32 * (1.0 - t) + y as f32 * t) as u32).min(0xff);
    mix(ch(a, 0), ch(b, 0)) | (mix(ch(a, 8), ch(b, 8)) << 8) | (mix(ch(a, 16), ch(b, 16)) << 16)
}

/// The client rectangle of one cell, in list coordinates.
unsafe fn subitem_rect(list: HWND, item: usize, col: usize) -> RECT {
    if col == 0 {
        let mut rc = RECT { left: LVIR_BOUNDS as i32, ..Default::default() };
        ui::send(list, LVM_GETITEMRECT, item, &mut rc as *mut _ as isize);
        rc.right = rc.left + ui::send(list, LVM_GETCOLUMNWIDTH, 0, 0) as i32;
        rc
    } else {
        let mut rc = RECT { left: LVIR_BOUNDS as i32, top: col as i32, ..Default::default() };
        ui::send(list, LVM_GETSUBITEMRECT, item, &mut rc as *mut _ as isize);
        rc
    }
}

/// Draws a filled rounded avatar chip with centered white initials.
unsafe fn draw_chip(hdc: HDC, x: i32, y: i32, sz: i32, color: u32, initials: &str, m: &ui::Metrics) {
    let r = m.px(5);
    let brush = CreateSolidBrush(COLORREF(color & 0x00FF_FFFF));
    let pen = CreatePen(PS_SOLID, 1, COLORREF(color & 0x00FF_FFFF));
    let ob = SelectObject(hdc, brush.into());
    let op = SelectObject(hdc, pen.into());
    let _ = RoundRect(hdc, x, y, x + sz, y + sz, r * 2, r * 2);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(0x00FF_FFFF));
    let mut rc = RECT { left: x, top: y, right: x + sz, bottom: y + sz };
    let mut text: Vec<u16> = initials.encode_utf16().collect();
    DrawTextW(hdc, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

/// Draws a small filled status dot centred on a point.
unsafe fn draw_dot(hdc: HDC, cx: i32, cy: i32, r: i32, color: u32) {
    let brush = CreateSolidBrush(COLORREF(color));
    let pen = CreatePen(PS_SOLID, 1, COLORREF(color));
    let ob = SelectObject(hdc, brush.into());
    let op = SelectObject(hdc, pen.into());
    let _ = Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());
}

/// Draws a subtle key outline: the cookie-stored marker.
unsafe fn draw_key(hdc: HDC, rc: &RECT, color: u32, m: &ui::Metrics) {
    let cy = (rc.top + rc.bottom) / 2;
    let r = m.px(3);
    let bx = rc.left + r + m.px(1);
    let pen = CreatePen(PS_SOLID, m.px(1).max(1), COLORREF(color));
    let op = SelectObject(hdc, pen.into());
    let hollow = GetStockObject(HOLLOW_BRUSH);
    let ob = SelectObject(hdc, hollow);
    let _ = Ellipse(hdc, bx - r, cy - r, bx + r, cy + r);
    let _ = MoveToEx(hdc, bx + r, cy, None);
    let _ = LineTo(hdc, rc.right, cy);
    let _ = MoveToEx(hdc, rc.right - m.px(2), cy, None);
    let _ = LineTo(hdc, rc.right - m.px(2), cy + m.px(3));
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(pen.into());
}

/// Draws one cell: an avatar chip, an online dot, a cookie marker and the
/// cell text, all tone-coloured.
unsafe fn draw_cell(app: &App, hdc: HDC, a: &Account, classic: bool, col: usize, rc: RECT) {
    let m = &app.m;
    let p = &app.palette;
    let pad = m.px(6);

    if classic && col == 0 {
        if matches!(app.clients.state(a.id), ClientState::Running) {
            let cx = (rc.left + rc.right) / 2;
            let cy = (rc.top + rc.bottom) / 2;
            draw_dot(hdc, cx, cy, m.px(4), tone_color(p, Tone::Good));
        }
        return;
    }

    let mut tx = rc.left + pad;
    if !classic && col == 1 {
        let h = rc.bottom - rc.top;
        let sz = (h - m.px(4)).clamp(m.px(14), m.px(20));
        let cy = rc.top + (h - sz) / 2;
        draw_chip(hdc, tx, cy, sz, avatars::color(a.id), &avatars::initials(a.display_name()), m);
        tx += sz + m.px(6);
    }

    let mut text_rc = RECT { left: tx, top: rc.top, right: rc.right - pad, bottom: rc.bottom };
    if !classic && col == 5 && app.cookie_state(a.id) == CookieState::Stored {
        let kw = m.px(16);
        let krc = RECT { left: text_rc.right - kw, top: rc.top, right: text_rc.right, bottom: rc.bottom };
        draw_key(hdc, &krc, p.muted, m);
        text_rc.right -= kw;
    }

    let text = cell(app, a, col as i32);
    if text.is_empty() {
        return;
    }
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(tone_color(p, cell_tone(app, a, classic, col))));
    let mut w: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut w, &mut text_rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
}

/// Paints one row: background, selection wash and bar, then every cell.
unsafe fn draw_row(app: &App, cd: &NMLVCUSTOMDRAW) {
    let list = app.accounts.list;
    let item = cd.nmcd.dwItemSpec;
    let Some(a) = app.accounts.rows.get(item).and_then(|id| app.store.account(*id)) else { return };
    let hdc = cd.nmcd.hdc;
    let p = &app.palette;

    let mut row_rc = RECT { left: LVIR_BOUNDS as i32, ..Default::default() };
    ui::send(list, LVM_GETITEMRECT, item, &mut row_rc as *mut _ as isize);
    let state = ui::send(list, LVM_GETITEMSTATE, item, (LVIS_SELECTED.0 | LVIS_FOCUSED.0) as isize) as u32;
    let selected = state & LVIS_SELECTED.0 != 0;
    let focused = state & LVIS_FOCUSED.0 != 0;

    let bg = if selected { blend(p.surface, p.accent, 0.22) } else { p.surface };
    let brush = CreateSolidBrush(COLORREF(bg));
    FillRect(hdc, &row_rc, brush);
    let _ = DeleteObject(brush.into());
    if selected {
        let bar = RECT { left: row_rc.left, top: row_rc.top, right: row_rc.left + app.m.px(3), bottom: row_rc.bottom };
        let ab = CreateSolidBrush(COLORREF(p.accent));
        FillRect(hdc, &bar, ab);
        let _ = DeleteObject(ab.into());
    }

    let classic = is_classic(app);
    let ncols = if classic { CLASSIC_COLUMNS.len() } else { MODERN_COLUMNS.len() };
    for col in 0..ncols {
        let rc = subitem_rect(list, item, col);
        draw_cell(app, hdc, a, classic, col, rc);
    }

    if focused {
        let fb = CreateSolidBrush(COLORREF(blend(p.surface, p.accent, 0.55)));
        FrameRect(hdc, &row_rc, fb);
        let _ = DeleteObject(fb.into());
    }
}

// ---- legend bar ------------------------------------------------------------

/// Subclasses the legend static so it owner-paints the counts and legend.
fn subclass_legend(legend: HWND) {
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(app) = app::try_app() {
                    paint_legend(app, hwnd, hdc);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
    unsafe {
        let _ = SetWindowSubclass(legend, Some(proc), 1, 0);
    }
}

/// Measures a string in the device context's current font.
unsafe fn text_width(hdc: HDC, text: &str) -> i32 {
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &w, &mut size);
    size.cx
}

unsafe fn draw_label(hdc: HDC, text: &str, rc: &mut RECT, color: u32, fmt: DRAW_TEXT_FORMAT) {
    SetTextColor(hdc, COLORREF(color));
    let mut w: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut w, rc, fmt | DT_SINGLELINE | DT_NOPREFIX);
}

/// Paints the counts on the left and, in the modern layout, a tone legend on
/// the right. The legend yields to the counts when the row is narrow.
fn paint_legend(app: &App, hwnd: HWND, hdc: HDC) {
    let rc = ui::client_rect(hwnd);
    let p = &app.palette;
    let m = &app.m;
    unsafe {
        let brush = CreateSolidBrush(COLORREF(p.window));
        FillRect(hdc, &rc, brush);
        let _ = DeleteObject(brush.into());
        SelectObject(hdc, m.font.into());
        SetBkMode(hdc, TRANSPARENT);
    }
    let running = app.clients.running_accounts().len();
    let total = app.store.accounts.len();
    let pad = m.px(2);

    if is_classic(app) {
        let text = format!("Total: {total}   \u{00B7}   Running: {running}");
        let mut trc = RECT { left: rc.left + pad, top: rc.top, right: rc.right, bottom: rc.bottom };
        unsafe { draw_label(hdc, &text, &mut trc, p.muted, DT_LEFT | DT_VCENTER) };
        return;
    }

    let selected = selected_ids(app).len();
    let cookies = app.store.accounts.iter().filter(|a| app.cookie_state(a.id) == CookieState::Stored).count();
    let counts = format!("{selected} selected   \u{00B7}   {total} accounts   \u{00B7}   {running} running   \u{00B7}   {cookies} with cookie");
    let mut crc = RECT { left: rc.left + pad, top: rc.top, right: rc.right, bottom: rc.bottom };
    unsafe { draw_label(hdc, &counts, &mut crc, p.muted, DT_LEFT | DT_VCENTER) };

    let legend = [
        ("Verified", p.good),
        ("Signed out", p.bad),
        ("Needs verification", p.warn),
        ("Running", p.good),
    ];
    let dot = m.px(8);
    let item_gap = m.px(14);
    let dot_gap = m.px(4);
    let (counts_w, labels_w) = unsafe {
        let cw = text_width(hdc, &counts);
        let lw: i32 = legend.iter().map(|(l, _)| text_width(hdc, l) + dot + dot_gap + item_gap).sum();
        (cw, lw)
    };
    // Choose the richest legend that fits without touching the counts.
    let cy = (rc.top + rc.bottom) / 2;
    let fits_labels = rc.left + pad + counts_w + m.px(16) + labels_w <= rc.right;
    let dots_only_w: i32 = legend.len() as i32 * (dot + item_gap);
    let fits_dots = rc.left + pad + counts_w + m.px(16) + dots_only_w <= rc.right;
    unsafe {
        if fits_labels {
            let mut x = rc.right - labels_w + item_gap / 2;
            for (label, color) in legend {
                draw_dot(hdc, x + dot / 2, cy, dot / 2, color);
                x += dot + dot_gap;
                let w = text_width(hdc, label);
                let mut lrc = RECT { left: x, top: rc.top, right: x + w, bottom: rc.bottom };
                draw_label(hdc, label, &mut lrc, p.muted, DT_LEFT | DT_VCENTER);
                x += w + item_gap;
            }
        } else if fits_dots {
            let mut x = rc.right - dots_only_w + item_gap / 2;
            for (_, color) in legend {
                draw_dot(hdc, x + dot / 2, cy, dot / 2, color);
                x += dot + item_gap;
            }
        }
    }
}

// ---- commands --------------------------------------------------------------

fn edit_account(app: &mut App, f: impl FnOnce(&mut Account)) {
    let Some(id) = app.accounts.shown else { return };
    if let Some(a) = app.store.account_mut(id) {
        f(a);
    }
    app.rebuild_index();
    app.mark_dirty();
    ui::invalidate(app.accounts.list);
}

pub fn command(app: &mut App, id: u16, code: u16, _ctl: HWND) {
    if app.accounts.loading {
        return;
    }
    const EN_CHANGE_CODE: u16 = 0x0300;
    const CBN_SELCHANGE_CODE: u16 = 1;
    const CBN_EDITCHANGE_CODE: u16 = 5;
    match id {
        ID_SEARCH if code == EN_CHANGE_CODE => unsafe {
            // Coalesces fast typing into one refresh.
            SetTimer(Some(app.hwnd), TIMER_SEARCH, 40, None);
        },
        ID_ALIAS if code == EN_CHANGE_CODE => {
            let text = ui::get_text(app.accounts.alias);
            edit_account(app, |a| a.alias = text.trim().to_string());
            let title = app.accounts.shown.map(|id| app.account_name(id)).unwrap_or_default();
            ui::set_text_if_changed(app.accounts.title, &title);
        }
        ID_TAGS if code == EN_CHANGE_CODE => {
            let text = ui::get_text(app.accounts.tags);
            edit_account(app, |a| {
                a.tags = text.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
            });
        }
        ID_NOTES if code == EN_CHANGE_CODE => {
            let text = ui::get_text(app.accounts.notes).replace("\r\n", "\n");
            edit_account(app, |a| a.notes = text);
        }
        ID_GROUP if code == CBN_EDITCHANGE_CODE => {
            let text = ui::get_text(app.accounts.group);
            edit_account(app, |a| a.group = text.trim().to_string());
        }
        ID_GROUP if code == CBN_SELCHANGE_CODE => {
            if let Some(i) = ui::combo_selected(app.accounts.group) {
                if let Some(g) = app.store.groups().get(i).cloned() {
                    edit_account(app, |a| a.group = g);
                }
            }
        }
        ID_FAV if code == 0 => toggle_favorite(app),
        ID_MAIN if code == 0 => toggle_main(app),
        CMD_TOGGLE_FAVORITE => toggle_favorite(app),
        CMD_TOGGLE_MAIN => toggle_main(app),
        ID_BROWSER if code == CBN_SELCHANGE_CODE => {
            let index = ui::combo_selected(app.accounts.browser).unwrap_or(0);
            if index == 0 {
                edit_account(app, |a| a.browser = None);
            }
            if let Some(a) = app.accounts.shown.and_then(|id| app.store.account(id)).cloned() {
                refresh_profiles(app, &a);
            }
        }
        ID_PROFILE if code == CBN_SELCHANGE_CODE => {
            let browser_index = ui::combo_selected(app.accounts.browser).unwrap_or(0);
            let profile = ui::combo_selected(app.accounts.profile).and_then(|i| app.accounts.profiles.get(i).cloned());
            if let (true, Some(p)) = (browser_index > 0, profile) {
                let browser = app.accounts.installs[browser_index - 1].id.clone();
                edit_account(app, |a| {
                    a.browser = Some(BrowserBinding { browser, profile: p.key.clone(), profile_name: p.display_name.clone() })
                });
            }
        }
        ID_PERF if code == CBN_SELCHANGE_CODE => {
            let choice = match ui::combo_selected(app.accounts.perf) {
                Some(1) => Some(PresetKind::Off),
                Some(2) => Some(PresetKind::Balanced),
                Some(3) => Some(PresetKind::LowResource),
                _ => None,
            };
            edit_account(app, |a| a.perf_override = choice);
            app.apply_perf();
            refresh_rows(app);
        }
        ID_ADD => add_account(app),
        ID_IMPORT => run_import(app),
        ID_REMOVE => remove_selected(app),
        ID_LAUNCH => launch_selected(app),
        ID_CANCEL => app.cancel_queue(),
        ID_JOIN => classic_quick_launch(app),
        ID_ACCOUNTS_FILE => shell_open(app, &app.data_dir.display().to_string()),
        ID_OPEN_LOG => {
            let path = app.log.path().display().to_string();
            shell_open(app, &path);
        }
        ID_COOKIE_SET => {
            if let Some(id) = single(app) {
                app.set_cookie(id);
                refresh_details(app);
            }
        }
        ID_COOKIE_REVEAL => {
            if let Some(id) = single(app) {
                app.reveal_cookie(id);
            }
        }
        ID_COOKIE_COPY => {
            if let Some(id) = single(app) {
                app.copy_cookie(id);
            }
        }
        ID_COOKIE_REMOVE => {
            if let Some(id) = single(app) {
                app.remove_cookie(id);
                refresh_details(app);
            }
        }
        ID_RECOVER => {
            if let Some(id) = single(app) {
                let state = app.store.account(id).map(|a| a.sign_in.state).unwrap_or_default();
                match signin::recovery(state).kind {
                    RecoveryKind::OpenSignIn => app.open_sign_in(id, signin::SIGN_IN_URL, "Sign-in page"),
                    RecoveryKind::OpenRoblox => app.open_sign_in(id, signin::HOME_URL, "Roblox"),
                    RecoveryKind::Recheck => app.check_sign_in(id),
                }
            }
        }
        ID_CHECK => {
            for id in selected_ids(app) {
                app.check_sign_in(id);
            }
        }
        ID_AUTHORIZE => {
            if let Some(id) = single(app) {
                app.authorize(id);
            }
        }
        ID_NEEDS_VERIFY => {
            if let Some(id) = single(app) {
                mark_verification(app, id);
            }
        }
        ID_RESUME => {
            if let Some(id) = single(app) {
                // The user says verification is done. The manager cannot
                // confirm that, so the state becomes "Check in browser".
                if let Some(a) = app.store.account_mut(id) {
                    if signin::blocks_launch(a.sign_in.state) {
                        signin::apply(&mut a.sign_in, &CheckOutcome::HandedOff { what: "Verification".into() }, now());
                    }
                }
                app.clients.resume(id);
                app.mark_dirty();
                app.pump_queue();
            }
        }
        ID_FOCUS_CLIENT => {
            if let Some(link) = single(app).and_then(|id| app.clients.link_for_account(id)) {
                if let Err(e) = app.windows.focus(link.process) {
                    app.set_status(&format!("Could not focus the client: {e}."));
                }
            }
        }
        ID_REJOIN => {
            if let Some(id) = single(app) {
                if let Some(target) = app.clients.last_target(id).cloned() {
                    app.launch(&[id], &target);
                }
            }
        }
        CMD_FOCUS_SEARCH => {
            if app.view != app::ViewId::Accounts {
                app.switch_view(app::ViewId::Accounts);
            }
            ui::focus(app.accounts.search);
            ui::send(app.accounts.search, EM_SETSEL, 0, -1);
        }
        CMD_SELECT_ALL => select_all(app),
        CMD_SIGN_IN => {
            if let Some(id) = single(app) {
                app.open_sign_in(id, signin::SIGN_IN_URL, "Sign-in page");
            }
        }
        CMD_RECOVERY => {
            if let Some(id) = single(app) {
                app.open_sign_in(id, signin::RECOVERY_URL, "Account recovery page");
            }
        }
        CMD_DEAUTHORIZE => {
            if let Some(id) = single(app) {
                app.remove_authorization(id);
            }
        }
        _ => {}
    }
}

/// Opens a file or folder with its default handler. Never fails hard.
fn shell_open(app: &App, path: &str) {
    let wide = ui::wide(path);
    // A failure (a value of 32 or below) is left silent on purpose.
    let _ = unsafe { ShellExecuteW(Some(app.hwnd), w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL) };
}

/// Classic quick-launch: launches the selection to the pasted place/job, or
/// to the destination combo when the fields are empty.
fn classic_quick_launch(app: &mut App) {
    let ids = selected_ids(app);
    if ids.is_empty() {
        app.set_status("Select one or more accounts to launch.");
        return;
    }
    let place = ui::get_text(app.accounts.place_edit).trim().to_string();
    let job = ui::get_text(app.accounts.job_edit).trim().to_string();
    let target = if place.is_empty() {
        match selected_target(app) {
            Some(t) => t,
            None => {
                app.set_status("Enter a place ID or choose a destination to launch to.");
                return;
            }
        }
    } else {
        let input = if job.is_empty() {
            place.clone()
        } else {
            format!("https://www.roblox.com/games/start?placeId={place}&gameInstanceId={job}")
        };
        match links::parse_destination(&input) {
            Ok(t) => t,
            Err(e) => {
                app.set_status(&e.to_string());
                return;
            }
        }
    };
    let targets = if ids.len() > 1 { search::batch_targets(&app.store, &ids) } else { ids.clone() };
    if ids.len() > 1 && targets.len() != ids.len() {
        app.set_status("Main was left out of the batch.");
    }
    app.launch(&targets, &target);
}

/// The user reports that Roblox asked for verification: the state changes
/// and any queued launch for the account pauses until they resume it.
fn mark_verification(app: &mut App, id: AccountId) {
    if let Some(a) = app.store.account_mut(id) {
        let outcome = CheckOutcome::VerificationRequired {
            reason: "You reported that Roblox asked for verification. Complete it in the browser, then resume.".into(),
        };
        signin::apply(&mut a.sign_in, &outcome, now());
    }
    app.clients.pause(id, "Complete verification in the browser, then resume.");
    app.mark_dirty();
    refresh_rows(app);
}

fn toggle_favorite(app: &mut App) {
    let ids = selected_ids(app);
    if ids.is_empty() {
        return;
    }
    let all = ids.iter().all(|id| app.store.account(*id).map(|a| a.favorite).unwrap_or(false));
    for id in ids {
        if let Some(a) = app.store.account_mut(id) {
            a.favorite = !all;
        }
    }
    app.rebuild_index();
    app.mark_dirty();
    refresh_rows(app);
}

fn toggle_main(app: &mut App) {
    let Some(id) = single(app) else {
        app.set_status("Select one account to pin as Main.");
        return;
    };
    app.store.main_account = if app.store.is_main(id) { None } else { Some(id) };
    let name = app.account_name(id);
    app.set_status(&match app.store.main_account {
        Some(_) => format!("{name} is now Main: excluded from Select All and batch actions, exempt from automatic performance changes."),
        None => "No account is pinned as Main.".to_string(),
    });
    // A client that just became Main gets its original settings back.
    app.apply_perf();
    app.mark_dirty();
    refresh(app);
}

fn select_all(app: &mut App) {
    // In a text box Ctrl+A selects the text.
    let focus = ui::focused();
    let mut class = [0u16; 16];
    let n = unsafe { GetClassNameW(focus, &mut class) };
    if String::from_utf16_lossy(&class[..n.max(0) as usize]).eq_ignore_ascii_case("edit") {
        ui::send(focus, EM_SETSEL, 0, -1);
        return;
    }
    if app.view != app::ViewId::Accounts {
        return;
    }
    let chosen = search::select_all(&app.store, &app.accounts.rows);
    app.accounts.loading = true;
    ui::list_set_selected(app.accounts.list, None, false);
    for (i, id) in app.accounts.rows.iter().enumerate() {
        if chosen.contains(id) {
            ui::list_set_selected(app.accounts.list, Some(i), true);
        }
    }
    app.accounts.loading = false;
    refresh_details(app);
    let skipped = app.accounts.rows.len() - chosen.len();
    app.set_status(&if skipped > 0 {
        format!("Selected {} accounts. Main is not included.", chosen.len())
    } else {
        format!("Selected {} accounts.", chosen.len())
    });
}

fn launch_selected(app: &mut App) {
    let ids = selected_ids(app);
    let Some(target) = selected_target(app) else {
        app.set_status("Choose a destination to launch to.");
        return;
    };
    // One account launches as asked, even Main. Several accounts are a
    // batch, which leaves Main out unless Settings says otherwise.
    let targets = if ids.len() > 1 { search::batch_targets(&app.store, &ids) } else { ids.clone() };
    if ids.len() > 1 && targets.len() != ids.len() {
        app.set_status("Main was left out of the batch.");
    }
    app.launch(&targets, &target);
}

fn add_account(app: &mut App) {
    let existing: Vec<String> = app.store.accounts.iter().map(|a| a.username.to_lowercase()).collect();
    let fields = [
        Field::line("Roblox username", "", "Exactly as it appears on Roblox"),
        Field::line("Alias (optional)", "", "A name only you see"),
    ];
    let Some(values) = dialogs::prompt(
        app.hwnd,
        "Add account",
        "The manager stores the name only. You sign in on roblox.com in your own browser; no password or cookie is saved here.",
        "Add",
        &fields,
        &|v| {
            let name = v[0].trim();
            if name.len() < 3 || name.len() > 20 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                Err("Usernames are 3\u{2013}20 letters, digits or underscores.".into())
            } else if existing.contains(&name.to_lowercase()) {
                Err("That account is already in the list.".into())
            } else {
                Ok(())
            }
        },
    ) else {
        return;
    };
    let id = app.store.alloc_id();
    app.store.accounts.push(Account {
        id,
        username: values[0].trim().to_string(),
        alias: values[1].trim().to_string(),
        created: now(),
        ..Default::default()
    });
    app.rebuild_index();
    app.save_now();
    ui::set_text(app.accounts.search, "");
    refresh(app);
    if let Some(row) = app.accounts.rows.iter().position(|r| *r == id) {
        ui::list_set_selected(app.accounts.list, None, false);
        ui::list_focus_row(app.accounts.list, row);
    }
    app.set_status("Account added. Use \"Sign in in browser\" to sign in with the browser profile you choose.");
}

fn remove_selected(app: &mut App) {
    let ids = selected_ids(app);
    if ids.is_empty() {
        return;
    }
    let text = if ids.len() == 1 {
        format!("Remove {} from the list?\n\nThis only removes the entry and its saved authorization from this PC. The Roblox account and the browser session are not affected.", app.account_name(ids[0]))
    } else {
        format!("Remove {} accounts from the list?\n\nThis only removes the entries and their saved authorizations from this PC.", ids.len())
    };
    if !ui::confirm(app.hwnd, "Remove account", &text) {
        return;
    }
    use ram_core::platform::SecretStore;
    let secrets = ram_win::WinSecrets::new();
    for id in &ids {
        let _ = secrets.delete(&ram_core::oauth::secret_name(*id));
        app.clients.forget_account(*id);
        if app.store.is_main(*id) {
            app.store.main_account = None;
        }
    }
    app.store.accounts.retain(|a| !ids.contains(&a.id));
    app.store.recent.retain(|r| !ids.contains(&r.account));
    app.rebuild_index();
    app.apply_perf();
    app.save_now();
    refresh(app);
    app.set_status(&format!("Removed {} account{}.", ids.len(), if ids.len() == 1 { "" } else { "s" }));
}

struct Dpapi;

impl LegacyDecryptor for Dpapi {
    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        ram_win::dpapi::unprotect(data, entropy).map_err(|e| e.to_string())
    }
}

/// Preview-first import from a legacy Roblox Account Manager folder.
fn run_import(app: &mut App) {
    let Some(folder) = dialogs::pick_folder(app.hwnd, "Choose the Roblox Account Manager folder") else { return };
    let scan = import::scan(&folder, Some(&Dpapi));
    let mut plan = import::plan(&app.store, &scan);
    let (add, update, unchanged, skip) = plan.counts();
    let mut intro = format!(
        "{add} to add, {update} to update, {unchanged} already present, {skip} skipped. Nothing in the source folder is changed. Passwords and cookies are not imported: every imported account needs a fresh browser sign-in."
    );
    for n in plan.notices.iter().take(3) {
        intro.push_str("\r\n\u{2022} ");
        intro.push_str(n);
    }
    if plan.items.is_empty() {
        ui::info(app.hwnd, "Import", &format!("Nothing to import from this folder.\n\n{}", plan.notices.join("\n")));
        return;
    }
    let rows: Vec<CheckRow> = plan
        .items
        .iter()
        .map(|item| {
            let kind = match item.kind {
                PlanKind::Account => "Account",
                PlanKind::Destination => "Destination",
                PlanKind::Game => "Game",
            };
            let (action, enabled) = match &item.action {
                PlanAction::Add => ("Add".to_string(), true),
                PlanAction::Update { changes } => (format!("Update {}", changes.join(", ")), true),
                PlanAction::Unchanged => ("Already present".to_string(), false),
                PlanAction::Skip { reason } => (format!("Skip: {reason}"), false),
            };
            CheckRow {
                label: format!("{kind}: {}", item.label),
                action,
                detail: item.detail.clone(),
                checked: item.selected,
                enabled,
            }
        })
        .collect();
    let Some(ticks) = dialogs::checklist(app.hwnd, "Import preview", &intro, "Import", &rows) else {
        app.set_status("Import cancelled. Nothing was changed.");
        return;
    };
    for (item, tick) in plan.items.iter_mut().zip(ticks) {
        item.selected = tick;
    }
    let record = import::apply(&mut app.store, &plan, now());
    app.store.settings.last_import_dir = folder.display().to_string();
    app.rebuild_index();
    app.save_now();
    refresh(app);
    crate::destinations::refresh(app);
    app.set_status(&format!(
        "Imported {} account(s), updated {}, added {} destination(s), skipped {}. Imported accounts need a fresh browser sign-in.",
        record.accounts_added, record.accounts_updated, record.destinations_added, record.skipped
    ));
}

// ---- notifications ---------------------------------------------------------

pub fn notify(app: &mut App, hdr: &NMHDR, lparam: LPARAM) -> Option<LRESULT> {
    if hdr.idFrom as u16 != ID_LIST {
        return None;
    }
    match hdr.code {
        LVN_GETDISPINFOW => unsafe {
            let info = &mut *(lparam.0 as *mut NMLVDISPINFOW);
            if info.item.mask.contains(LVIF_TEXT) {
                let text = app
                    .accounts
                    .rows
                    .get(info.item.iItem as usize)
                    .and_then(|id| app.store.account(*id))
                    .map(|a| cell(app, a, info.item.iSubItem))
                    .unwrap_or_default();
                ui::fill_dispinfo(&mut info.item, &text);
            }
            Some(LRESULT(0))
        },
        LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
            // Selection changes arrive in bursts (and continuously while an
            // arrow key is held); the details pane updates once they settle.
            if !app.accounts.loading {
                ui::invalidate(app.accounts.legend);
                unsafe {
                    SetTimer(Some(app.hwnd), TIMER_DETAILS, 15, None);
                }
            }
            Some(LRESULT(0))
        }
        LVN_COLUMNCLICK => unsafe {
            let info = &*(lparam.0 as *const NMLISTVIEW);
            let key = if is_classic(app) {
                match info.iSubItem {
                    0 => SortKey::Default,
                    1 => SortKey::Username,
                    2 => SortKey::Name,
                    3 => SortKey::Group,
                    4 => SortKey::SignIn,
                    _ => return Some(LRESULT(0)),
                }
            } else {
                match info.iSubItem {
                    0 => SortKey::Default,
                    1 => SortKey::Name,
                    2 => SortKey::Username,
                    3 => SortKey::Group,
                    5 => SortKey::SignIn,
                    7 => SortKey::LastUsed,
                    _ => return Some(LRESULT(0)),
                }
            };
            if app.accounts.sort == key {
                app.accounts.descending = !app.accounts.descending;
            } else {
                app.accounts.sort = key;
                app.accounts.descending = false;
            }
            refresh(app);
            Some(LRESULT(0))
        },
        NM_DBLCLK => {
            launch_selected(app);
            Some(LRESULT(0))
        }
        LVN_KEYDOWN => unsafe {
            let info = &*(lparam.0 as *const NMLVKEYDOWN);
            if info.wVKey == VK_DELETE.0 {
                remove_selected(app);
            }
            Some(LRESULT(0))
        },
        LVN_ODFINDITEMW => unsafe {
            // Type-to-select: jumps to the next row whose name starts with
            // what was typed.
            let info = &*(lparam.0 as *const NMLVFINDITEMW);
            let rows = &app.accounts.rows;
            if rows.is_empty() || info.lvfi.psz.is_null() || (info.lvfi.flags.0 & (LVFI_STRING.0 | LVFI_PARTIAL.0)) == 0 {
                return Some(LRESULT(-1));
            }
            let needle = info.lvfi.psz.to_string().unwrap_or_default().to_lowercase();
            let start = info.iStart.max(0) as usize % rows.len();
            for offset in 0..rows.len() {
                let i = (start + offset) % rows.len();
                let hit = app.store.account(rows[i]).map(|a| a.display_name().to_lowercase().starts_with(&needle));
                if hit == Some(true) {
                    return Some(LRESULT(i as isize));
                }
            }
            Some(LRESULT(-1))
        },
        LVN_GETEMPTYMARKUP => unsafe {
            let info = &mut *(lparam.0 as *mut NMLVEMPTYMARKUP);
            let text = if app.store.accounts.is_empty() {
                "No accounts yet. Press Ctrl+N to add one, or Ctrl+I to import from Roblox Account Manager."
            } else {
                "No accounts match this search. Press Esc to clear it."
            };
            info.dwFlags = EMF_CENTERED;
            for (i, unit) in text.encode_utf16().take(info.szMarkup.len() - 1).enumerate() {
                info.szMarkup[i] = unit;
                info.szMarkup[i + 1] = 0;
            }
            Some(LRESULT(1))
        },
        NM_CUSTOMDRAW => unsafe {
            let cd = &mut *(lparam.0 as *mut NMLVCUSTOMDRAW);
            const PREPAINT: u32 = 0x1;
            const ITEM_PREPAINT: u32 = 0x10001;
            match cd.nmcd.dwDrawStage.0 {
                PREPAINT => Some(LRESULT(CDRF_NOTIFYITEMDRAW as isize)),
                ITEM_PREPAINT => {
                    draw_row(app, cd);
                    Some(LRESULT(CDRF_SKIPDEFAULT as isize))
                }
                _ => Some(LRESULT(0)),
            }
        },
        _ => None,
    }
}

/// Shows the account context menu at a screen position; (-1, -1) means it
/// was opened from the keyboard and appears at the focused row.
pub fn context_menu(app: &mut App, x: i32, y: i32) {
    let ids = selected_ids(app);
    if ids.is_empty() {
        return;
    }
    let one = ids.len() == 1;
    let linked = one && app.clients.link_for_account(ids[0]).is_some();
    let can_rejoin = one && !linked && app.clients.last_target(ids[0]).is_some();
    let is_main = one && app.store.is_main(ids[0]);
    let items: [(u16, &str, bool); 14] = [
        (ID_LAUNCH, "Launch\tEnter", true),
        (ID_REJOIN, "Rejoin last destination\tCtrl+R", can_rejoin),
        (ID_FOCUS_CLIENT, "Focus client\tF6", linked),
        (0, "", false),
        (CMD_SIGN_IN, "Sign in in browser", one),
        (CMD_RECOVERY, "Open account recovery page", one),
        (ID_CHECK, "Check sign-in\tF5", true),
        (ID_AUTHORIZE, "Authorize verified checks\u{2026}", one),
        (CMD_DEAUTHORIZE, "Remove authorization", one),
        (ID_NEEDS_VERIFY, "Mark as needing verification", one),
        (0, "", false),
        (CMD_TOGGLE_FAVORITE, "Favorite\tCtrl+D", true),
        (CMD_TOGGLE_MAIN, if is_main { "Unpin Main\tCtrl+M" } else { "Pin as Main\tCtrl+M" }, one),
        (ID_REMOVE, "Remove\u{2026}\tDel", true),
    ];
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for (id, text, enabled) in items {
            if id == 0 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            } else {
                let flags = if enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                let _ = AppendMenuW(menu, flags, id as usize, windows::core::PCWSTR(ui::wide(text).as_ptr()));
            }
        }
        let (mut px, mut py) = (x, y);
        if x == -1 && y == -1 {
            let mut rc = RECT { left: LVIR_LABEL as i32, ..Default::default() };
            let row = ui::list_selected(app.accounts.list).first().copied().unwrap_or(0);
            ui::send(app.accounts.list, LVM_GETITEMRECT, row, &mut rc as *mut _ as isize);
            let mut pt = POINT { x: rc.left + app.m.px(24), y: rc.bottom };
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(app.accounts.list, &mut pt);
            (px, py) = (pt.x, pt.y);
        }
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, px, py, None, app.hwnd, None);
        let _ = DestroyMenu(menu);
        if chosen.0 != 0 {
            command(app, chosen.0 as u16, 0, HWND::default());
        }
    }
}

/// Keys the dialog manager would otherwise swallow. Returns true when the
/// message was handled.
pub fn pre_translate(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    let Some(app) = app::try_app() else { return false };
    if app.view != app::ViewId::Accounts {
        return false;
    }
    let key = msg.wParam.0 as u16;
    let (list, search, place_edit, job_edit) =
        (app.accounts.list, app.accounts.search, app.accounts.place_edit, app.accounts.job_edit);
    if key == VK_RETURN.0 {
        if msg.hwnd == place_edit || msg.hwnd == job_edit {
            classic_quick_launch(app);
            return true;
        }
        if msg.hwnd == list {
            launch_selected(app);
            return true;
        }
        if msg.hwnd == search {
            if !app.accounts.rows.is_empty() && selected_ids(app).is_empty() {
                ui::list_focus_row(app.accounts.list, 0);
            }
            ui::focus(app.accounts.list);
            return true;
        }
    }
    if key == VK_ESCAPE.0 {
        if !ui::get_text(search).is_empty() {
            ui::set_text(search, "");
            ui::focus(app.accounts.search);
            return true;
        }
        if app.clients.queued_len() > 0 {
            app.cancel_queue();
            return true;
        }
    }
    false
}

/// Measures table responsiveness with the current store. Used by
/// `--bench-ui`; results are written to a file by the caller.
pub fn bench(app: &mut App) -> String {
    let n = app.store.accounts.len();
    let mut out = format!("accounts={n}\n");
    let time = |f: &mut dyn FnMut()| {
        let t = Instant::now();
        f();
        t.elapsed().as_secs_f64() * 1000.0
    };
    out.push_str(&format!("index_build_ms={:.3}\n", time(&mut || app.rebuild_index())));
    let queries = ["", "demo", "alias 4", "tag:gaia", "group:farm is:fav", "zzz-no-match"];
    for q in queries {
        ui::set_text(app.accounts.search, q);
        let ms = time(&mut || {
            refresh(app);
            unsafe {
                let _ = UpdateWindow(app.hwnd);
            }
        });
        out.push_str(&format!("search_refresh_paint_ms[{q:?}]={ms:.3} rows={}\n", app.accounts.rows.len()));
    }
    ui::set_text(app.accounts.search, "");
    refresh(app);
    // Keyboard navigation: move the focused row through the whole list.
    let rows = app.accounts.rows.len().min(200);
    let ms = time(&mut || {
        for i in 0..rows {
            ui::list_set_selected(app.accounts.list, None, false);
            ui::list_focus_row(app.accounts.list, i);
            unsafe {
                let _ = UpdateWindow(app.hwnd);
            }
        }
    });
    out.push_str(&format!("select_row_and_paint_ms_per_row={:.4}\n", ms / rows.max(1) as f64));
    let ms = time(&mut || {
        for i in 0..rows {
            ui::list_set_selected(app.accounts.list, None, false);
            ui::list_focus_row(app.accounts.list, i);
            refresh_details(app);
            unsafe {
                let _ = UpdateWindow(app.hwnd);
            }
        }
    });
    out.push_str(&format!("select_row_details_and_paint_ms_per_row={:.4}\n", ms / rows.max(1) as f64));
    let ms = time(&mut || {
        for _ in 0..20 {
            ui::invalidate(app.accounts.list);
            unsafe {
                let _ = UpdateWindow(app.accounts.list);
            }
        }
    });
    out.push_str(&format!("full_table_repaint_ms={:.3}\n", ms / 20.0));
    out
}
