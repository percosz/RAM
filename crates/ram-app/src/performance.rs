//! The Performance view: the optional preset for background clients, the
//! table of running clients, and the advisory, reminder and sampling
//! settings.
//!
//! Everything here is off by default and reversible. The view changes a
//! client only through `App::apply_perf`, window focus and tiling, and a
//! normal close request the user confirms.

use crate::app::{tone_color, try_app, App, Tone, ViewId, LABEL, LABEL_MUTED};
use crate::dialogs;
use crate::ui::{self, Metrics};
use ram_core::model::{AccountId, PresetKind};
use ram_core::perf;
use ram_core::platform::{ProcKey, ProcessApi, WindowApi};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_LIST: u16 = 1200;
pub const ID_RESET: u16 = 1201;
const ID_PRESET: u16 = 1202;
const ID_APPLY_MAIN: u16 = 1203;
const ID_TILE_ON_LAUNCH: u16 = 1204;
const ID_CPU_CHECK: u16 = 1205;
const ID_CPU_EDIT: u16 = 1206;
const ID_ASSIGN: u16 = 1210;
const ID_UNASSIGN: u16 = 1211;
const ID_FOCUS: u16 = 1212;
const ID_TILE: u16 = 1213;
const ID_CLOSE: u16 = 1214;
const ID_WARN_PRIVATE: u16 = 1220;
const ID_WARN_SYSTEM: u16 = 1221;
const ID_NOTIFY: u16 = 1222;
const ID_IDLE: u16 = 1223;
const ID_SAMPLE_VISIBLE: u16 = 1224;
const ID_SAMPLE_HIDDEN: u16 = 1225;
const ID_OVERRIDES: u16 = 1230;
const ID_MEMORY: u16 = 1231;

/// Timer on the main window that applies a typed CPU limit once the user
/// pauses. It has its own callback, so the main window never sees it.
const TIMER_CPU_APPLY: usize = 1250;

const DASH: &str = "\u{2014}";
const PRESETS: [PresetKind; 3] = [PresetKind::Off, PresetKind::Balanced, PresetKind::LowResource];
const DEFAULT_CPU_CAP: u32 = 25;
const FACTS_MAX_AGE: Duration = Duration::from_secs(30);

const T_PRESET: &str = "Background clients";
const T_STANDING: &str = "Main and clients not linked to an account are never changed.";
const T_APPLY_MAIN: &str = "Also apply to Main";
const T_TILE_ON_LAUNCH: &str = "Tile clients when one starts";
const T_CPU_CHECK: &str = "Limit CPU with a Job Object";
const T_CPU_SUFFIX: &str = "% of total CPU";
const T_CPU_CAVEAT: &str = "Hard limit across all cores; applied only where Windows allows it, removed on reset or exit. The limit is a share of total CPU, so 25% on 8 cores is 2 cores.";
const T_WARN_PRIVATE: &str = "Warn when a client's private memory exceeds";
const T_WARN_PRIVATE_UNIT: &str = "MB (0 = off)";
const T_WARN_SYSTEM: &str = "Warn when system memory use exceeds";
const T_WARN_SYSTEM_UNIT: &str = "% (0 = off)";
const T_NOTIFY: &str = "Also show a notification";
const T_ADVISORY: &str = "Advisory only. RAM never trims or closes a client by itself.";
const T_IDLE: &str = "Remind me when a running client has not been focused for";
const T_IDLE_UNIT: &str = "minutes (0 = off)";
const T_IDLE_NOTE: &str = "Reminder only; RAM never sends input to a client. Roblox disconnects idle players after about 20 minutes.";
const T_SAMPLE: [&str; 3] = ["Sample every", "s while this view is open, every", "s otherwise"];
const T_FPS_REST: &str = "Roblox stores this once per Windows user, so it applies to every client including Main. RAM does not change it; set it in the client's Settings menu.";
const T_PER_CLIENT: &str = "Per-client FPS caps and graphics presets: unavailable \u{2014} Roblox exposes no supported per-client setting.";
const T_GPU_LIMIT: &str = "Hard GPU limit: unsupported \u{2014} Windows has no per-process GPU cap.";
const T_EMPTY: &str = "No Roblox clients are running. Launch an account from Accounts; its client appears here once Windows shows it.";

/// Table columns: title, width in DIPs (0 = takes the spare width), and
/// whether the values are right-aligned.
const COLUMNS: [(&str, i32, bool); 10] = [
    ("Account", 0, false),
    ("PID", 48, true),
    ("Preset", 140, false),
    ("Priority", 88, false),
    ("CPU %", 52, true),
    ("Working set", 78, true),
    ("Private", 66, true),
    ("GPU %", 52, true),
    ("GPU memory", 88, true),
    ("Status", 0, false),
];
const COL_ACCOUNT: i32 = 0;
const COL_PID: i32 = 1;
const COL_PRESET: i32 = 2;
const COL_STATUS: i32 = 9;

#[derive(Default)]
pub struct PerformanceView {
    pub list: HWND,
    preset_label: HWND,
    preset: HWND,
    preset_desc: HWND,
    standing: HWND,
    apply_main: HWND,
    tile_on_launch: HWND,
    cpu_check: HWND,
    cpu_edit: HWND,
    cpu_suffix: HWND,
    cpu_caveat: HWND,
    assign: HWND,
    unassign: HWND,
    focus: HWND,
    tile: HWND,
    close: HWND,
    reset: HWND,
    overrides: HWND,
    warn_private_label: HWND,
    warn_private: HWND,
    warn_private_unit: HWND,
    warn_system_label: HWND,
    warn_system: HWND,
    warn_system_unit: HWND,
    notify: HWND,
    memory: HWND,
    advisory: HWND,
    idle_label: HWND,
    idle: HWND,
    idle_unit: HWND,
    idle_note: HWND,
    sample_labels: [HWND; 3],
    sample_visible: HWND,
    sample_hidden: HWND,
    fps: HWND,
    per_client: HWND,
    gpu_limit: HWND,
    /// Client processes in display order; the virtual list reads from this.
    rows: Vec<ProcKey>,
    /// Set while the view updates its own controls, so the change
    /// notifications those updates raise are ignored.
    loading: bool,
    /// True once the view has refreshed while visible; cleared by a refresh
    /// that finds it hidden.
    shown: bool,
    /// A typed CPU limit is stored but not yet applied to the clients.
    cpu_pending: bool,
    /// DPI the fixed column widths were last set for.
    column_dpi: u32,
    /// System memory load from the last refresh, for the tone of that line.
    memory_load: Option<u32>,
    /// The frame-rate line and when Roblox's settings file was last read.
    fps_text: String,
    fps_read: Option<Instant>,
}

pub fn static_tone(app: &App, id: u16) -> Option<Tone> {
    match id {
        ID_OVERRIDES => Some(if perf::has_active_overrides(&app.store) { Tone::Text } else { Tone::Muted }),
        ID_MEMORY => {
            let limit = app.store.perf.system_load_warn_percent;
            let over = limit > 0 && app.performance.memory_load.is_some_and(|load| load >= limit);
            Some(if over { Tone::Warn } else { Tone::Muted })
        }
        _ => None,
    }
}

// ---- controls --------------------------------------------------------------

/// A whole-number edit sized for `digits` digits.
fn number_edit(parent: HWND, id: u16, digits: usize) -> HWND {
    const ES_NUMBER_STYLE: i32 = 0x2000;
    const EM_LIMITTEXT_MSG: u32 = 0x00C5;
    let h = ui::edit(parent, id);
    unsafe {
        let style = GetWindowLongW(h, GWL_STYLE);
        SetWindowLongW(h, GWL_STYLE, style | ES_NUMBER_STYLE);
    }
    ui::send(h, EM_LIMITTEXT_MSG, digits, 0);
    h
}

pub fn create(app: &mut App) {
    let p = app.panels[2];
    let v = &mut app.performance;
    // Controls are created in tab order: preset and options, the table,
    // its actions, then the settings below it.
    v.preset_label = ui::label(p, LABEL, T_PRESET);
    v.preset = ui::combo(p, ID_PRESET);
    for kind in PRESETS {
        ui::combo_add(v.preset, kind.label());
    }
    v.preset_desc = ui::paragraph(p, LABEL_MUTED, preset_description(PresetKind::Off));
    v.standing = ui::paragraph(p, LABEL_MUTED, T_STANDING);
    v.apply_main = ui::checkbox(p, ID_APPLY_MAIN, T_APPLY_MAIN);
    v.tile_on_launch = ui::checkbox(p, ID_TILE_ON_LAUNCH, T_TILE_ON_LAUNCH);
    v.cpu_check = ui::checkbox(p, ID_CPU_CHECK, T_CPU_CHECK);
    v.cpu_edit = number_edit(p, ID_CPU_EDIT, 3);
    v.cpu_suffix = ui::label(p, LABEL, T_CPU_SUFFIX);
    v.cpu_caveat = ui::paragraph(p, LABEL_MUTED, T_CPU_CAVEAT);

    v.list = ui::listview(p, ID_LIST, ui::LIST_STYLE_SINGLE);
    for (i, (title, width, right)) in COLUMNS.iter().enumerate() {
        ui::list_add_column(v.list, i as i32, title, app.m.px((*width).max(96)));
        if *right {
            let col = LVCOLUMNW { mask: LVCF_FMT, fmt: LVCFMT_RIGHT, ..Default::default() };
            ui::send(v.list, LVM_SETCOLUMNW, i, &col as *const _ as isize);
        }
    }
    crate::theme::subclass_list(v.list);

    v.assign = ui::button(p, ID_ASSIGN, "Assign to account\u{2026}");
    v.unassign = ui::button(p, ID_UNASSIGN, "Unassign");
    v.focus = ui::button(p, ID_FOCUS, "Focus");
    v.tile = ui::button(p, ID_TILE, "Tile identified clients");
    v.close = ui::button(p, ID_CLOSE, "Close client\u{2026}");
    v.overrides = ui::label(p, ID_OVERRIDES, "");
    unsafe {
        // Right-aligned, so the count sits against the reset button.
        const SS_RIGHT_STYLE: i32 = 0x2;
        let style = GetWindowLongW(v.overrides, GWL_STYLE);
        SetWindowLongW(v.overrides, GWL_STYLE, style | SS_RIGHT_STYLE);
    }
    v.reset = ui::button(p, ID_RESET, "Reset overrides");

    v.warn_private_label = ui::label(p, LABEL, T_WARN_PRIVATE);
    v.warn_private = number_edit(p, ID_WARN_PRIVATE, 7);
    v.warn_private_unit = ui::label(p, LABEL, T_WARN_PRIVATE_UNIT);
    v.warn_system_label = ui::label(p, LABEL, T_WARN_SYSTEM);
    v.warn_system = number_edit(p, ID_WARN_SYSTEM, 3);
    v.warn_system_unit = ui::label(p, LABEL, T_WARN_SYSTEM_UNIT);
    v.notify = ui::checkbox(p, ID_NOTIFY, T_NOTIFY);
    v.memory = ui::label(p, ID_MEMORY, "");
    v.advisory = ui::paragraph(p, LABEL_MUTED, T_ADVISORY);
    v.idle_label = ui::label(p, LABEL, T_IDLE);
    v.idle = number_edit(p, ID_IDLE, 4);
    v.idle_unit = ui::label(p, LABEL, T_IDLE_UNIT);
    v.idle_note = ui::paragraph(p, LABEL_MUTED, T_IDLE_NOTE);

    v.sample_labels[0] = ui::label(p, LABEL, T_SAMPLE[0]);
    v.sample_visible = number_edit(p, ID_SAMPLE_VISIBLE, 2);
    v.sample_labels[1] = ui::label(p, LABEL, T_SAMPLE[1]);
    v.sample_hidden = number_edit(p, ID_SAMPLE_HIDDEN, 3);
    v.sample_labels[2] = ui::label(p, LABEL, T_SAMPLE[2]);
    v.fps_text = fps_line(None);
    v.fps = ui::paragraph(p, LABEL_MUTED, &v.fps_text);
    v.per_client = ui::paragraph(p, LABEL_MUTED, T_PER_CLIENT);
    v.gpu_limit = ui::paragraph(p, LABEL_MUTED, T_GPU_LIMIT);
}

// ---- layout ----------------------------------------------------------------

/// Measures text in the UI font.
struct Measure {
    hwnd: HWND,
    hdc: HDC,
    old: HGDIOBJ,
}

impl Measure {
    fn new(hwnd: HWND, font: HFONT) -> Self {
        unsafe {
            let hdc = GetDC(Some(hwnd));
            let old = SelectObject(hdc, font.into());
            Measure { hwnd, hdc, old }
        }
    }

    fn size(&self, text: &str) -> SIZE {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            let _ = GetTextExtentPoint32W(self.hdc, &wide, &mut size);
        }
        size
    }

    fn width(&self, text: &str) -> i32 {
        self.size(text).cx
    }

    fn line_height(&self) -> i32 {
        self.size("Ag").cy
    }

    /// Height of `text` wrapped to `width`.
    fn wrapped_height(&self, text: &str, width: i32) -> i32 {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT { left: 0, top: 0, right: width.max(1), bottom: 0 };
        let h = unsafe { DrawTextW(self.hdc, &mut wide, &mut rc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX) };
        h.max(self.line_height())
    }
}

impl Drop for Measure {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            ReleaseDC(Some(self.hwnd), self.hdc);
        }
    }
}

/// One control in a row of a `Column`.
enum Item<'a> {
    /// A single-line label as wide as its text.
    Text(HWND, &'a str),
    /// A check box as wide as its text.
    Check(HWND, &'a str),
    /// A number edit of the given DIP width.
    Edit(HWND, i32),
    /// A drop-down of the given DIP width.
    Combo(HWND, i32),
    /// A label that takes the rest of the row.
    Fill(HWND),
}

/// Stacks rows and paragraphs top to bottom in a fixed-width column and
/// records where each control goes. Rows wrap when the column is too narrow.
struct Column<'a> {
    ms: &'a Measure,
    m: &'a Metrics,
    x: i32,
    w: i32,
    y: i32,
    out: Vec<(HWND, i32, i32, i32, i32)>,
}

impl<'a> Column<'a> {
    fn new(ms: &'a Measure, m: &'a Metrics, x: i32, w: i32) -> Self {
        Column { ms, m, x, w, y: 0, out: Vec::new() }
    }

    fn line(&mut self, items: &[Item]) {
        let (m, row) = (self.m, self.m.row());
        let text_h = self.ms.line_height() + m.px(2);
        let right = self.x + self.w;
        let mut cx = self.x;
        for (i, item) in items.iter().enumerate() {
            let wanted = match item {
                Item::Text(_, text) => self.ms.width(text) + m.px(2),
                Item::Check(_, text) => self.ms.width(text) + m.px(24),
                Item::Edit(_, dip) | Item::Combo(_, dip) => m.px(*dip),
                Item::Fill(_) => (right - cx).max(m.px(120)),
            };
            let w = wanted.min(self.w);
            // An edit stays on the same row as the unit that follows it.
            let unit_w = match (item, items.get(i + 1)) {
                (Item::Edit(..), Some(Item::Text(_, unit))) => m.px(8) + self.ms.width(unit),
                _ => 0,
            };
            if cx > self.x && cx + w + unit_w > right {
                cx = self.x;
                self.y += row;
            }
            let y = self.y;
            self.out.push(match item {
                Item::Text(h, _) | Item::Fill(h) => (*h, cx, y + (row - text_h) / 2, w, text_h),
                Item::Check(h, _) => (*h, cx, y, w, row),
                Item::Edit(h, _) => (*h, cx, y + m.px(2), w, row - m.px(4)),
                Item::Combo(h, _) => (*h, cx, y + m.px(2), w, row * 8),
            });
            cx += w + m.px(6);
        }
        self.y += row;
    }

    fn paragraph(&mut self, h: HWND, text: &str) {
        let height = self.ms.wrapped_height(text, self.w);
        self.out.push((h, self.x, self.y, self.w, height));
        self.y += height + self.m.px(3);
    }

    fn space(&mut self, dip: i32) {
        self.y += self.m.px(dip);
    }

    fn place(&self, dy: i32) {
        for (h, x, y, w, height) in &self.out {
            ui::place(*h, *x, *y + dy, *w, *height);
        }
    }
}

pub fn layout(app: &mut App) {
    let panel = app.panels[2];
    let rc = ui::client_rect(panel);
    if rc.right <= 0 || rc.bottom <= 0 {
        return;
    }
    let m = &app.m;
    let v = &app.performance;
    let (gap, row) = (m.gap(), m.row());
    let ms = Measure::new(panel, m.font);
    let inner = rc.right - 2 * gap;
    let column_gap = m.px(16);
    let col_w = (inner - column_gap) / 2;
    let right_x = gap + col_w + column_gap;

    // Top: preset on the left, options on the right.
    let mut left = Column::new(&ms, m, gap, col_w);
    left.line(&[Item::Text(v.preset_label, T_PRESET), Item::Combo(v.preset, 150)]);
    // Room for the longest description, so choosing a preset moves nothing.
    left.paragraph(v.preset_desc, preset_description(PresetKind::LowResource));
    left.paragraph(v.standing, T_STANDING);
    let mut right = Column::new(&ms, m, right_x, col_w);
    right.line(&[Item::Check(v.apply_main, T_APPLY_MAIN), Item::Check(v.tile_on_launch, T_TILE_ON_LAUNCH)]);
    right.line(&[Item::Check(v.cpu_check, T_CPU_CHECK), Item::Edit(v.cpu_edit, 40), Item::Text(v.cpu_suffix, T_CPU_SUFFIX)]);
    right.paragraph(v.cpu_caveat, T_CPU_CAVEAT);
    left.place(0);
    right.place(0);
    let top_h = left.y.max(right.y);

    // Bottom: advisories and reminders on the left, sampling and the
    // frame-rate facts on the right.
    let mut left = Column::new(&ms, m, gap, col_w);
    left.line(&[
        Item::Text(v.warn_private_label, T_WARN_PRIVATE),
        Item::Edit(v.warn_private, 64),
        Item::Text(v.warn_private_unit, T_WARN_PRIVATE_UNIT),
    ]);
    left.line(&[
        Item::Text(v.warn_system_label, T_WARN_SYSTEM),
        Item::Edit(v.warn_system, 40),
        Item::Text(v.warn_system_unit, T_WARN_SYSTEM_UNIT),
    ]);
    left.line(&[Item::Check(v.notify, T_NOTIFY), Item::Fill(v.memory)]);
    left.paragraph(v.advisory, T_ADVISORY);
    left.space(4);
    left.line(&[Item::Text(v.idle_label, T_IDLE), Item::Edit(v.idle, 48), Item::Text(v.idle_unit, T_IDLE_UNIT)]);
    left.paragraph(v.idle_note, T_IDLE_NOTE);
    let mut right = Column::new(&ms, m, right_x, col_w);
    right.line(&[
        Item::Text(v.sample_labels[0], T_SAMPLE[0]),
        Item::Edit(v.sample_visible, 34),
        Item::Text(v.sample_labels[1], T_SAMPLE[1]),
        Item::Edit(v.sample_hidden, 40),
        Item::Text(v.sample_labels[2], T_SAMPLE[2]),
    ]);
    right.space(4);
    right.paragraph(v.fps, &v.fps_text);
    right.paragraph(v.per_client, T_PER_CLIENT);
    right.paragraph(v.gpu_limit, T_GPU_LIMIT);
    let bottom_h = left.y.max(right.y);
    let bottom_y = rc.bottom - gap - bottom_h;
    left.place(bottom_y);
    right.place(bottom_y);

    // Action row above the bottom area: client actions from the left, the
    // override count and the reset button from the right.
    let buttons_y = bottom_y - gap - row;
    let button_w = |h: HWND| (ms.width(&ui::get_text(h)) + m.px(24)).max(m.px(72));
    let mut x = gap;
    for b in [v.assign, v.unassign, v.focus, v.tile, v.close] {
        let w = button_w(b);
        ui::place(b, x, buttons_y, w, row);
        x += w + gap;
    }
    let reset_w = button_w(v.reset);
    let reset_x = (rc.right - gap - reset_w).max(x);
    ui::place(v.reset, reset_x, buttons_y, reset_w, row);
    let text_h = ms.line_height() + m.px(2);
    ui::place(v.overrides, x, buttons_y + (row - text_h) / 2, reset_x - gap - x, text_h);

    // The table takes whatever height is left.
    let list_y = top_h + gap;
    ui::place(v.list, gap, list_y, inner, buttons_y - gap - list_y);
    let fixed: i32 = COLUMNS.iter().map(|(_, w, _)| m.px(*w)).sum();
    let spare = inner - fixed - m.px(20);
    let account_w = (spare * 2 / 5).clamp(m.px(100), m.px(220));
    let status_w = (spare - account_w).max(m.px(96));
    ui::send(v.list, LVM_SETCOLUMNWIDTH, COL_ACCOUNT as usize, account_w as isize);
    ui::send(v.list, LVM_SETCOLUMNWIDTH, COL_STATUS as usize, status_w as isize);
    // Fixed columns are sized once per DPI, so widths the user drags stay.
    if v.column_dpi != m.dpi {
        for (i, (_, w, _)) in COLUMNS.iter().enumerate().filter(|(_, c)| c.1 > 0) {
            ui::send(v.list, LVM_SETCOLUMNWIDTH, i, m.px(*w) as isize);
        }
    }
    let dpi = m.dpi;
    drop(ms);
    app.performance.column_dpi = dpi;
}

// ---- data ------------------------------------------------------------------

fn preset_description(kind: PresetKind) -> &'static str {
    match kind {
        PresetKind::Off => "No automatic changes.",
        PresetKind::Balanced => "Below-normal priority for identified background clients.",
        PresetKind::LowResource => "Low priority and Windows efficiency mode for identified background clients.",
    }
}

/// Finds the `FramerateCap` value in the text of Roblox's settings file.
fn framerate_cap(xml: &str) -> Option<i64> {
    let rest = &xml[xml.find("name=\"FramerateCap\"")?..];
    let rest = &rest[rest.find('>')? + 1..];
    rest[..rest.find('<')?].trim().parse().ok()
}

/// Reads the frame-rate cap Roblox saved for this Windows user. `None` when
/// the file or the value is missing.
fn read_framerate_cap() -> Option<i64> {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let path = ram_win::paths::roblox_global_settings_file()?;
    if std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    framerate_cap(&String::from_utf8_lossy(&bytes))
}

fn fps_line(cap: Option<i64>) -> String {
    let value = match cap {
        None => "not set".to_string(),
        Some(n) if n <= 0 => "default".to_string(),
        Some(n) => format!("{n} FPS"),
    };
    format!("Frame rate cap: {value}. {T_FPS_REST}")
}

fn selected_key(app: &App) -> Option<ProcKey> {
    ui::list_selected(app.performance.list).first().and_then(|i| app.performance.rows.get(*i).copied())
}

fn linked_account(app: &App, key: ProcKey) -> Option<AccountId> {
    app.clients.link_for_process(key).map(|l| l.account)
}

/// Name used for a client in messages: its account, or its process ID.
fn client_name(app: &App, key: ProcKey) -> String {
    match linked_account(app, key) {
        Some(id) => format!("{}'s client (process {})", app.account_name(id), key.pid),
        None => format!("the unassigned client (process {})", key.pid),
    }
}

fn megabytes(bytes: u64) -> String {
    ui::mb(bytes)
}

fn cell(app: &App, key: ProcKey, column: i32) -> String {
    let link = app.clients.link_for_process(key);
    let sampled = app.samples.get(&key);
    let dash = || DASH.to_string();
    match column {
        COL_ACCOUNT => match link {
            Some(l) if app.store.is_main(l.account) => format!("{} (Main)", app.account_name(l.account)),
            Some(l) => app.account_name(l.account),
            None => "Unassigned".to_string(),
        },
        COL_PID => key.pid.to_string(),
        COL_PRESET => perf::effective(&app.store, link.map(|l| l.account)).label(),
        3 => sampled.and_then(|s| s.sample.priority).map(|p| p.label().to_string()).unwrap_or_else(dash),
        4 => sampled.and_then(|s| s.cpu_percent).map(|p| format!("{p:.1}")).unwrap_or_else(dash),
        5 => sampled.map(|s| megabytes(s.sample.working_set)).unwrap_or_else(dash),
        6 => sampled.map(|s| megabytes(s.sample.private_bytes)).unwrap_or_else(dash),
        7 => sampled.and_then(|s| s.sample.gpu_percent).map(|p| format!("{p:.1}")).unwrap_or_else(dash),
        8 => sampled.and_then(|s| s.sample.gpu_dedicated_bytes).map(megabytes).unwrap_or_else(dash),
        COL_STATUS => {
            let mut text = match (app.perf_notes.get(&key), link) {
                (Some(note), _) => note.clone(),
                (None, Some(l)) => {
                    let label = l.evidence.label();
                    let mut chars = label.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                        None => String::new(),
                    }
                }
                (None, None) => "Not linked to an account".to_string(),
            };
            if app.store.applied.iter().any(|e| e.process == key && e.cpu_cap) {
                text.push_str(" \u{00B7} CPU limited");
            }
            text
        }
        _ => String::new(),
    }
}

// ---- numeric settings ------------------------------------------------------

const NUMBER_FIELDS: [u16; 6] = [ID_CPU_EDIT, ID_WARN_PRIVATE, ID_WARN_SYSTEM, ID_IDLE, ID_SAMPLE_VISIBLE, ID_SAMPLE_HIDDEN];

/// Accepted range of a numeric setting.
fn field_range(id: u16) -> (u32, u32) {
    match id {
        ID_CPU_EDIT => (1, 100),
        ID_WARN_PRIVATE => (0, 1_048_576),
        ID_WARN_SYSTEM => (0, 100),
        ID_IDLE => (0, 1440),
        ID_SAMPLE_VISIBLE => (1, 60),
        _ => (5, 600),
    }
}

fn field_edit(v: &PerformanceView, id: u16) -> HWND {
    match id {
        ID_CPU_EDIT => v.cpu_edit,
        ID_WARN_PRIVATE => v.warn_private,
        ID_WARN_SYSTEM => v.warn_system,
        ID_IDLE => v.idle,
        ID_SAMPLE_VISIBLE => v.sample_visible,
        _ => v.sample_hidden,
    }
}

/// The stored value, or `None` for the CPU limit while it is switched off.
fn field_value(app: &App, id: u16) -> Option<u32> {
    let perf = &app.store.perf;
    match id {
        ID_CPU_EDIT => perf.cpu_cap_percent,
        ID_WARN_PRIVATE => Some(perf.client_private_warn_mb),
        ID_WARN_SYSTEM => Some(perf.system_load_warn_percent),
        ID_IDLE => Some(app.store.settings.idle_reminder_minutes),
        ID_SAMPLE_VISIBLE => Some(perf.sample_secs_visible),
        _ => Some(perf.sample_secs_hidden),
    }
}

fn set_field_value(app: &mut App, id: u16, value: u32) {
    let perf = &mut app.store.perf;
    match id {
        ID_CPU_EDIT => perf.cpu_cap_percent = Some(value),
        ID_WARN_PRIVATE => perf.client_private_warn_mb = value,
        ID_WARN_SYSTEM => perf.system_load_warn_percent = value,
        ID_IDLE => app.store.settings.idle_reminder_minutes = value,
        ID_SAMPLE_VISIBLE => perf.sample_secs_visible = value,
        _ => perf.sample_secs_hidden = value,
    }
}

/// The number typed into a field, clamped to its range. `None` while the
/// text is empty or not a number.
fn typed_value(app: &App, id: u16) -> Option<u32> {
    let (min, max) = field_range(id);
    let text = ui::get_text(field_edit(&app.performance, id));
    text.trim().parse::<u64>().ok().map(|n| n.clamp(min as u64, max as u64) as u32)
}

/// Stores what the user typed, if it is a number. The text is left alone so
/// typing is never interrupted.
fn field_changed(app: &mut App, id: u16) {
    let Some(value) = typed_value(app, id) else { return };
    let current = field_value(app, id);
    // A switched-off CPU limit stays off whatever its edit holds.
    if current == Some(value) || current.is_none() {
        return;
    }
    set_field_value(app, id, value);
    app.mark_dirty();
    match id {
        ID_CPU_EDIT => {
            // Applied once typing pauses, so "50" does not pass through 5%.
            app.performance.cpu_pending = true;
            unsafe {
                SetTimer(Some(app.hwnd), TIMER_CPU_APPLY, 700, Some(cpu_timer));
            }
        }
        ID_WARN_SYSTEM => {
            ui::invalidate(app.performance.memory);
            app.update_timers();
        }
        _ => app.update_timers(),
    }
}

/// Shows the stored, clamped value once the user leaves a field.
fn field_committed(app: &mut App, id: u16) {
    if let Some(value) = field_value(app, id) {
        app.performance.loading = true;
        ui::set_text_if_changed(field_edit(&app.performance, id), &value.to_string());
        app.performance.loading = false;
    }
    if id == ID_CPU_EDIT {
        apply_pending_cpu(app);
    }
}

unsafe extern "system" fn cpu_timer(_hwnd: HWND, _msg: u32, _id: usize, _time: u32) {
    if let Some(app) = try_app() {
        apply_pending_cpu(app);
    }
}

fn apply_pending_cpu(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_CPU_APPLY);
    }
    if std::mem::take(&mut app.performance.cpu_pending) {
        app.apply_perf();
        refresh(app);
    }
}

// ---- refresh ---------------------------------------------------------------

/// Brings the view up to date with the clients, samples and settings. Does
/// nothing while the view is hidden; switching to it refreshes it.
pub fn refresh(app: &mut App) {
    if app.view != ViewId::Performance {
        app.performance.shown = false;
        return;
    }
    let entering = !std::mem::replace(&mut app.performance.shown, true);
    refresh_rows(app);
    sync_controls(app);
    refresh_actions(app);
    refresh_summary(app);
    let stale = app.performance.fps_read.is_none_or(|at| at.elapsed() >= FACTS_MAX_AGE);
    if entering || stale {
        refresh_facts(app);
    }
}

/// Updates the table rows, keeping the selected client selected.
fn refresh_rows(app: &mut App) {
    let keys: Vec<ProcKey> = app.clients.processes().iter().map(|p| p.key).collect();
    if keys == app.performance.rows {
        ui::invalidate(app.performance.list);
        return;
    }
    let selected = selected_key(app);
    let v = &mut app.performance;
    v.loading = true;
    ui::list_set_selected(v.list, None, false);
    v.rows = keys;
    ui::list_set_count(v.list, v.rows.len());
    if let Some(i) = selected.and_then(|key| v.rows.iter().position(|k| *k == key)) {
        ui::list_focus_row(v.list, i);
    }
    v.loading = false;
}

fn set_check(h: HWND, on: bool) {
    if ui::is_checked(h) != on {
        ui::set_checked(h, on);
    }
}

/// Copies the settings into the controls. An edit the user is typing in is
/// left alone.
fn sync_controls(app: &mut App) {
    app.performance.loading = true;
    let v = &app.performance;
    let perf = &app.store.perf;
    let index = PRESETS.iter().position(|p| *p == perf.preset).unwrap_or(0);
    if ui::combo_selected(v.preset) != Some(index) {
        ui::combo_select(v.preset, index as i32);
    }
    ui::set_text_if_changed(v.preset_desc, preset_description(perf.preset));
    set_check(v.apply_main, perf.apply_to_main);
    set_check(v.tile_on_launch, perf.tile_on_launch);
    set_check(v.cpu_check, perf.cpu_cap_percent.is_some());
    set_check(v.notify, perf.notify_on_threshold);
    let focused = ui::focused();
    for id in NUMBER_FIELDS {
        let edit = field_edit(v, id);
        if edit == focused {
            continue;
        }
        if let Some(value) = field_value(app, id) {
            ui::set_text_if_changed(edit, &value.to_string());
        }
    }
    set_enabled(v, v.cpu_edit, perf.cpu_cap_percent.is_some());
    app.performance.loading = false;
}

/// Enables or disables a control. Focus moves to the table first when the
/// control being disabled has it, so the keyboard is never left nowhere.
fn set_enabled(v: &PerformanceView, h: HWND, on: bool) {
    if !on && ui::focused() == h {
        ui::focus(v.list);
    }
    ui::enable(h, on);
}

/// True when the reset would change anything.
fn reset_applies(app: &App) -> bool {
    perf::has_active_overrides(&app.store)
        || app.store.perf.preset != PresetKind::Off
        || app.store.perf.cpu_cap_percent.is_some()
        || app.store.accounts.iter().any(|a| a.perf_override.is_some())
}

/// Enables the actions that apply to the selected client.
fn refresh_actions(app: &mut App) {
    let selected = selected_key(app);
    let linked = selected.and_then(|key| linked_account(app, key)).is_some();
    let any_linked = app.performance.rows.iter().any(|key| linked_account(app, *key).is_some());
    let v = &app.performance;
    set_enabled(v, v.assign, selected.is_some() && !linked);
    set_enabled(v, v.unassign, linked);
    set_enabled(v, v.focus, selected.is_some());
    set_enabled(v, v.close, selected.is_some());
    set_enabled(v, v.tile, any_linked);
    set_enabled(v, v.reset, reset_applies(app));
}

/// Updates the override count and the system memory line.
fn refresh_summary(app: &mut App) {
    let text = match app.store.applied.len() {
        0 => "No overrides active".to_string(),
        1 => "1 override active".to_string(),
        n => format!("{n} overrides active"),
    };
    ui::set_text_if_changed(app.performance.overrides, &text);

    const GB: f64 = (1024u64 * 1024 * 1024) as f64;
    let memory = app.procs.system_memory().ok();
    let text = match memory {
        Some(mem) => format!(
            "Memory: {:.1} of {:.1} GB in use ({}%)",
            mem.total_physical.saturating_sub(mem.available_physical) as f64 / GB,
            mem.total_physical as f64 / GB,
            mem.load_percent
        ),
        None => format!("Memory: {DASH}"),
    };
    app.performance.memory_load = memory.map(|mem| mem.load_percent);
    ui::set_text_if_changed(app.performance.memory, &text);
}

/// Re-reads the frame-rate cap from Roblox's settings file.
fn refresh_facts(app: &mut App) {
    app.performance.fps_read = Some(Instant::now());
    let text = fps_line(read_framerate_cap());
    if text != app.performance.fps_text {
        ui::set_text_if_changed(app.performance.fps, &text);
        app.performance.fps_text = text;
        // The line may now wrap differently.
        layout(app);
    }
}

// ---- commands --------------------------------------------------------------

pub fn command(app: &mut App, id: u16, code: u16, _ctl: HWND) {
    if app.performance.loading {
        return;
    }
    const CLICKED: u16 = 0;
    const ACCELERATOR: u16 = 1;
    const CBN_SELCHANGE_CODE: u16 = 1;
    const EN_KILLFOCUS_CODE: u16 = 0x0200;
    const EN_CHANGE_CODE: u16 = 0x0300;
    match id {
        ID_RESET if code == CLICKED || code == ACCELERATOR => reset_overrides(app),
        ID_PRESET if code == CBN_SELCHANGE_CODE => {
            let kind = ui::combo_selected(app.performance.preset).and_then(|i| PRESETS.get(i).copied());
            if let Some(kind) = kind.filter(|k| *k != app.store.perf.preset) {
                app.store.perf.preset = kind;
                settings_changed(app);
                app.set_status(&format!("Background clients: {}. {}", kind.label(), preset_description(kind)));
            }
        }
        ID_APPLY_MAIN if code == CLICKED => {
            app.store.perf.apply_to_main = ui::is_checked(app.performance.apply_main);
            settings_changed(app);
        }
        ID_TILE_ON_LAUNCH if code == CLICKED => {
            app.store.perf.tile_on_launch = ui::is_checked(app.performance.tile_on_launch);
            settings_changed(app);
        }
        ID_CPU_CHECK if code == CLICKED => {
            app.store.perf.cpu_cap_percent = if ui::is_checked(app.performance.cpu_check) {
                Some(typed_value(app, ID_CPU_EDIT).unwrap_or(DEFAULT_CPU_CAP))
            } else {
                None
            };
            app.performance.cpu_pending = false;
            settings_changed(app);
        }
        ID_NOTIFY if code == CLICKED => {
            app.store.perf.notify_on_threshold = ui::is_checked(app.performance.notify);
            app.mark_dirty();
        }
        _ if NUMBER_FIELDS.contains(&id) && code == EN_CHANGE_CODE => field_changed(app, id),
        _ if NUMBER_FIELDS.contains(&id) && code == EN_KILLFOCUS_CODE => field_committed(app, id),
        ID_ASSIGN if code == CLICKED => assign_selected(app),
        ID_UNASSIGN if code == CLICKED => unassign_selected(app),
        ID_FOCUS if code == CLICKED => focus_selected(app),
        ID_TILE if code == CLICKED => tile_clients(app),
        ID_CLOSE if code == CLICKED => close_selected(app),
        _ => {}
    }
}

/// Applies a changed preset or option to the running clients.
fn settings_changed(app: &mut App) {
    app.apply_perf();
    app.mark_dirty();
    refresh(app);
}

/// Restores every client, then switches off whatever would change one
/// again: the preset, the CPU limit and the per-account presets.
fn reset_overrides(app: &mut App) {
    app.reset_perf();
    app.performance.cpu_pending = false;
    app.store.perf.preset = PresetKind::Off;
    app.store.perf.cpu_cap_percent = None;
    for account in app.store.accounts.iter_mut() {
        account.perf_override = None;
    }
    app.mark_dirty();
    let left = app.store.applied.len();
    app.set_status(&if left == 0 {
        "Original settings restored. The preset is Off, the CPU limit is off and per-account presets are cleared, so nothing is re-applied.".to_string()
    } else {
        format!("{left} override(s) could not be restored; see the Status column. The preset is Off, so nothing new is applied.")
    });
    if app.view == ViewId::Accounts {
        crate::accounts::refresh_rows(app);
    }
    refresh(app);
}

fn assign_selected(app: &mut App) {
    let Some(key) = selected_key(app) else { return };
    if linked_account(app, key).is_some() {
        return;
    }
    // Accounts without a client, Main first, then by name.
    let mut free: Vec<(AccountId, String)> = app
        .store
        .accounts
        .iter()
        .filter(|a| app.clients.link_for_account(a.id).is_none())
        .map(|a| {
            let name = a.display_name().to_string();
            (a.id, if app.store.is_main(a.id) { format!("{name} (Main)") } else { name })
        })
        .collect();
    free.sort_by_key(|(id, name)| (!app.store.is_main(*id), name.to_lowercase()));
    if free.is_empty() {
        app.set_status("Every account already has a client. Unassign one first, or add the account in Accounts.");
        return;
    }
    let names: Vec<String> = free.iter().map(|(_, name)| name.clone()).collect();
    let intro = format!(
        "Which account is signed in to the client with process ID {}? RAM cannot check this. If it may be Main, choose Main so it stays exempt.",
        key.pid
    );
    let picked = dialogs::choose(app.hwnd, "Assign to account", &intro, "Assign", &names);
    ui::focus(app.performance.list);
    let Some((account, name)) = picked.and_then(|i| free.get(i).cloned()) else { return };
    if !app.clients.assign(key, account) {
        app.set_status("That client is no longer running.");
        refresh(app);
        return;
    }
    settings_changed(app);
    app.set_status(&format!("Client {} is now assigned to {name}.", key.pid));
}

fn unassign_selected(app: &mut App) {
    let Some(key) = selected_key(app) else { return };
    let Some(account) = linked_account(app, key) else { return };
    let name = app.account_name(account);
    app.clients.unassign(key);
    // An unassigned client is exempt, so its original settings come back.
    settings_changed(app);
    app.set_status(&format!("Client {} is no longer linked to {name}. It is exempt from presets until assigned.", key.pid));
}

fn focus_selected(app: &mut App) {
    let Some(key) = selected_key(app) else { return };
    if let Err(e) = app.windows.focus(key) {
        app.set_status(&format!("Could not focus {}: {e}.", client_name(app, key)));
    }
}

fn tile_clients(app: &mut App) {
    let pairs = app.client_pairs();
    let identified = pairs.iter().filter(|(_, account)| account.is_some()).count();
    let notes = perf::tile(&mut app.store, &pairs, &app.windows);
    let failed = notes.len();
    for note in notes {
        app.perf_notes.insert(note.process, note.message);
    }
    app.mark_dirty();
    app.set_status(&match (identified, failed) {
        (0, _) => "No identified clients to tile. Assign a client to an account first.".to_string(),
        (_, 0) => "Tiled the windows of identified clients. Reset overrides puts them back.".to_string(),
        (_, n) => format!("{n} client window(s) could not be moved; see the Status column."),
    });
    refresh(app);
}

/// Asks the selected client to close after the user confirms. This is the
/// same request as clicking the window's close button.
fn close_selected(app: &mut App) {
    let Some(key) = selected_key(app) else { return };
    let who = client_name(app, key);
    let mut text = String::new();
    if let Some(account) = linked_account(app, key).filter(|id| app.store.is_main(*id)) {
        text.push_str(&format!("This client is linked to your Main account, {}.\n\n", app.account_name(account)));
    }
    text.push_str(&format!(
        "Ask {who} to close?\n\nThis asks the client to close, like clicking the X on its window. RAM never force-terminates a client: if Roblox does not close, it keeps running."
    ));
    let confirmed = ui::confirm(app.hwnd, "Close client", &text);
    ui::focus(app.performance.list);
    if !confirmed {
        return;
    }
    match app.procs.request_close(key) {
        Ok(()) => app.set_status(&format!("Asked {who} to close.")),
        Err(e) => app.set_status(&format!("Could not ask {who} to close: {e}.")),
    }
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
                    .performance
                    .rows
                    .get(info.item.iItem as usize)
                    .map(|key| cell(app, *key, info.item.iSubItem))
                    .unwrap_or_default();
                ui::fill_dispinfo(&mut info.item, &text);
            }
            Some(LRESULT(0))
        },
        LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
            if !app.performance.loading {
                refresh_actions(app);
            }
            Some(LRESULT(0))
        }
        NM_DBLCLK => {
            focus_selected(app);
            Some(LRESULT(0))
        }
        LVN_GETEMPTYMARKUP => unsafe {
            let info = &mut *(lparam.0 as *mut NMLVEMPTYMARKUP);
            info.dwFlags = EMF_CENTERED;
            for (i, unit) in T_EMPTY.encode_utf16().take(info.szMarkup.len() - 1).enumerate() {
                info.szMarkup[i] = unit;
                info.szMarkup[i + 1] = 0;
            }
            Some(LRESULT(1))
        },
        NM_CUSTOMDRAW => unsafe {
            let cd = &mut *(lparam.0 as *mut NMLVCUSTOMDRAW);
            const PREPAINT: u32 = 0x1;
            const ITEM_PREPAINT: u32 = 0x10001;
            const SUBITEM_PREPAINT: u32 = 0x30001;
            match cd.nmcd.dwDrawStage.0 {
                PREPAINT => Some(LRESULT(CDRF_NOTIFYITEMDRAW as isize)),
                ITEM_PREPAINT => Some(LRESULT(CDRF_NOTIFYSUBITEMDRAW as isize)),
                SUBITEM_PREPAINT => {
                    let key = app.performance.rows.get(cd.nmcd.dwItemSpec).copied();
                    let account = key.and_then(|k| linked_account(app, k));
                    let tone = match (cd.iSubItem, key) {
                        (COL_ACCOUNT, Some(_)) if account.is_none() => Tone::Warn,
                        (COL_ACCOUNT, Some(_)) if account.is_some_and(|id| app.store.is_main(id)) => Tone::Good,
                        (COL_PID, _) => Tone::Muted,
                        (COL_PRESET, Some(_)) => match perf::effective(&app.store, account) {
                            perf::Effective::Preset(PresetKind::Off) => Tone::Muted,
                            perf::Effective::Preset(_) => Tone::Text,
                            _ => Tone::Muted,
                        },
                        (COL_STATUS, Some(k)) if app.perf_notes.contains_key(&k) => Tone::Warn,
                        (COL_STATUS, _) => Tone::Muted,
                        _ => Tone::Text,
                    };
                    cd.clrText = COLORREF(tone_color(&app.palette, tone));
                    Some(LRESULT(CDRF_NEWFONT as isize))
                }
                _ => Some(LRESULT(0)),
            }
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_frame_rate_cap() {
        let xml = r#"<Item><Properties><int name="Fullscreen">0</int><int name="FramerateCap">144</int></Properties></Item>"#;
        assert_eq!(framerate_cap(xml), Some(144));
        assert_eq!(framerate_cap(r#"<int name="FramerateCap"> -1 </int>"#), Some(-1));
        assert_eq!(framerate_cap("<int name=\"Fullscreen\">0</int>"), None);
        assert_eq!(framerate_cap("<int name=\"FramerateCap\">"), None);
        assert_eq!(framerate_cap("<int name=\"FramerateCap\">fast</int>"), None);
    }

    #[test]
    fn describes_the_frame_rate_cap() {
        assert!(fps_line(None).starts_with("Frame rate cap: not set. Roblox stores"));
        assert!(fps_line(Some(-1)).starts_with("Frame rate cap: default. "));
        assert!(fps_line(Some(0)).starts_with("Frame rate cap: default. "));
        assert!(fps_line(Some(60)).starts_with("Frame rate cap: 60 FPS. "));
    }
}
