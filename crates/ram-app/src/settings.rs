//! The Settings view: appearance, launch pacing, the optional verified
//! sign-in setup, and data and diagnostics. Performance tuning has its own
//! view.

use crate::app::{App, LABEL, LABEL_MUTED};
use crate::ui;
use ram_core::model::ThemeKind;
use ram_core::oauth;
use ram_core::redact::redact;
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_THEME: u16 = 1300;
const ID_ACCENT: u16 = 1301;
const ID_ACCENT_RESET: u16 = 1302;
const ID_COMPACT: u16 = 1303;
const ID_CONCURRENCY: u16 = 1304;
const ID_SPACING: u16 = 1305;
const ID_TIMEOUT: u16 = 1306;
const ID_INCLUDE_MAIN: u16 = 1307;
const ID_CLIENT_ID: u16 = 1308;
const ID_PORT: u16 = 1309;
const ID_REDIRECT: u16 = 1310;
const ID_COPY_URL: u16 = 1311;
const ID_OPEN_DIR: u16 = 1312;
const ID_COPY_DIAG: u16 = 1313;
/// Hidden edit that holds text on its way to the clipboard.
const ID_CLIP: u16 = 1314;
/// Validation text under the accent field. Its own ID gives it the ordinary
/// text color, so it stands out from the muted hints.
pub const ID_ACCENT_NOTE: u16 = 1315;
const ID_LAYOUT: u16 = 1316;
const ID_DIRECT: u16 = 1317;
const ID_MULTI: u16 = 1318;
const ID_VAULT_SETUP: u16 = 1319;
const ID_VAULT_UNLOCK: u16 = 1320;
const ID_VAULT_LOCK: u16 = 1321;
const ID_VAULT_CHANGE: u16 = 1322;
const ID_VAULT_REMOVE: u16 = 1323;
const ID_VAULT_HELLO: u16 = 1324;
const ID_VAULT_AUTOLOCK: u16 = 1325;
const ID_EXPORT: u16 = 1326;

const EN_CHANGE_CODE: u16 = 0x0300;
const EN_KILLFOCUS_CODE: u16 = 0x0200;
const CBN_SELCHANGE_CODE: u16 = 1;
const CLICKED: u16 = 0;

// Themes come from ThemeKind::all(); the picker lists every one.

const ACCENT_INVALID: &str = "Use the form #RRGGBB.";
const LOG_TAIL_LINES: usize = 40;

#[link(name = "user32")]
extern "system" {
    fn GetClipboardSequenceNumber() -> u32;
}

#[derive(Default)]
pub struct SettingsView {
    /// The first focusable control; it gets focus when the view opens.
    pub first: HWND,
    sections: [HWND; 5],
    l_theme: HWND,
    theme: HWND,
    l_accent: HWND,
    accent: HWND,
    accent_reset: HWND,
    accent_note: HWND,
    compact: HWND,
    l_layout: HWND,
    layout: HWND,
    contrast_note: HWND,
    direct: HWND,
    multi: HWND,
    multi_note: HWND,
    // Security (cookie vault).
    vault_status: HWND,
    vault_setup: HWND,
    vault_unlock: HWND,
    vault_lock: HWND,
    vault_change: HWND,
    vault_remove: HWND,
    vault_hello: HWND,
    l_autolock: HWND,
    autolock: HWND,
    h_autolock: HWND,
    export_btn: HWND,
    l_concurrency: HWND,
    concurrency: HWND,
    h_concurrency: HWND,
    concurrency_note: HWND,
    l_spacing: HWND,
    spacing: HWND,
    h_spacing: HWND,
    l_timeout: HWND,
    timeout: HWND,
    h_timeout: HWND,
    include_main: HWND,
    main_note: HWND,
    oauth_note: HWND,
    l_client_id: HWND,
    client_id: HWND,
    l_port: HWND,
    port: HWND,
    h_port: HWND,
    l_redirect: HWND,
    redirect: HWND,
    copy_url: HWND,
    data_path: HWND,
    open_dir: HWND,
    copy_diag: HWND,
    about: HWND,
    clip: HWND,
    /// Set while the view updates its own controls or restyles the window,
    /// so the change notifications those updates raise are ignored.
    loading: bool,
}

pub fn create(app: &mut App) {
    let p = app.panels[3];
    let v = &mut app.settings;
    let muted = |text: &str| ui::label(p, LABEL_MUTED, text);
    let number = |id: u16, digits: usize| {
        let h = ui::edit(p, id);
        unsafe {
            let style = GetWindowLongW(h, GWL_STYLE);
            SetWindowLongW(h, GWL_STYLE, style | ES_NUMBER);
        }
        ui::send(h, EM_SETLIMITTEXT, digits, 0);
        h
    };

    v.sections[0] = muted("Appearance");
    v.l_theme = ui::label(p, LABEL, "Theme");
    v.theme = ui::combo(p, ID_THEME);
    for t in ThemeKind::all() {
        ui::combo_add(v.theme, t.label());
    }
    v.l_accent = ui::label(p, LABEL, "Accent color");
    v.accent = ui::edit(p, ID_ACCENT);
    ui::send(v.accent, EM_SETLIMITTEXT, 7, 0);
    ui::set_cue(v.accent, "Theme accent (#RRGGBB)");
    v.accent_reset = ui::button(p, ID_ACCENT_RESET, "Reset");
    v.accent_note = ui::label(p, ID_ACCENT_NOTE, "");
    v.compact = ui::checkbox(p, ID_COMPACT, "Compact density");
    v.l_layout = ui::label(p, LABEL, "Accounts layout");
    v.layout = ui::combo(p, ID_LAYOUT);
    ui::combo_add(v.layout, "Modern");
    ui::combo_add(v.layout, "Classic");
    v.contrast_note = ui::paragraph(p, LABEL_MUTED, "High-contrast mode always uses the Windows colors.");

    v.sections[1] = muted("Launching");
    v.l_concurrency = ui::label(p, LABEL, "Launches in flight at once");
    v.concurrency = number(ID_CONCURRENCY, 1);
    v.h_concurrency = muted("1\u{2013}4");
    v.concurrency_note = ui::paragraph(
        p,
        LABEL_MUTED,
        "With more than one launch in flight, a new client often cannot be matched to an account and stays unconfirmed.",
    );
    v.l_spacing = ui::label(p, LABEL, "Seconds between launches");
    v.spacing = number(ID_SPACING, 3);
    v.h_spacing = muted("0\u{2013}120");
    v.l_timeout = ui::label(p, LABEL, "Give up waiting for a client after");
    v.timeout = number(ID_TIMEOUT, 3);
    v.h_timeout = muted("s  (20\u{2013}600)");
    v.include_main = ui::checkbox(p, ID_INCLUDE_MAIN, "Include Main in Select All and batch actions");
    v.main_note =
        ui::paragraph(p, LABEL_MUTED, "Off by default: Main is only launched when you select it on its own.");
    v.direct = ui::checkbox(p, ID_DIRECT, "Launch directly with a stored cookie when the vault is unlocked");
    v.multi = ui::checkbox(p, ID_MULTI, "Run several Roblox clients at once (multi-instance)");
    v.multi_note = ui::paragraph(
        p,
        LABEL_MUTED,
        "Multi-instance holds Roblox's single-instance lock so more than one client can run. Roblox does not officially support this; it does not modify the client.",
    );

    v.sections[2] = muted("Security");
    v.vault_status = ui::paragraph(p, LABEL_MUTED, "");
    v.vault_setup = ui::button(p, ID_VAULT_SETUP, "Set up vault\u{2026}");
    v.vault_unlock = ui::button(p, ID_VAULT_UNLOCK, "Unlock");
    v.vault_lock = ui::button(p, ID_VAULT_LOCK, "Lock");
    v.vault_change = ui::button(p, ID_VAULT_CHANGE, "Change password\u{2026}");
    v.vault_remove = ui::button(p, ID_VAULT_REMOVE, "Remove\u{2026}");
    v.vault_hello = ui::checkbox(p, ID_VAULT_HELLO, "Require Windows Hello to reveal or copy a cookie");
    v.l_autolock = ui::label(p, LABEL, "Lock after idle");
    v.autolock = number(ID_VAULT_AUTOLOCK, 4);
    v.h_autolock = muted("min (0 = never)");

    v.sections[3] = muted("Verified sign-in (optional)");
    v.oauth_note = ui::paragraph(
        p,
        LABEL_MUTED,
        "Roblox can confirm which account a browser profile is signed in to through its OAuth 2.0 sign-in. \
         Create an OAuth app in the Roblox Creator Dashboard with the scopes openid and profile and the \
         redirect URL below, then paste its client ID here. Without it, sign-in status stays 'Check in \
         browser'. The client ID is not a secret; authorizations are kept in Windows Credential Manager.",
    );
    v.l_client_id = ui::label(p, LABEL, "Client ID");
    v.client_id = ui::edit(p, ID_CLIENT_ID);
    ui::send(v.client_id, EM_SETLIMITTEXT, 128, 0);
    v.l_port = ui::label(p, LABEL, "Local redirect port");
    v.port = number(ID_PORT, 5);
    v.h_port = muted("1024\u{2013}65535");
    v.l_redirect = ui::label(p, LABEL, "Redirect URL");
    v.redirect = ui::edit(p, ID_REDIRECT);
    ui::send(v.redirect, EM_SETREADONLY, 1, 0);
    v.copy_url = ui::button(p, ID_COPY_URL, "Copy");

    v.sections[4] = muted("Data and diagnostics");
    v.data_path = muted(&format!("Data folder: {}", app.data_dir.display()));
    v.open_dir = ui::button(p, ID_OPEN_DIR, "Open data folder");
    v.copy_diag = ui::button(p, ID_COPY_DIAG, "Copy diagnostics");
    v.export_btn = ui::button(p, ID_EXPORT, "Export to AccountData.json\u{2026}");
    v.about = ui::paragraph(
        p,
        LABEL_MUTED,
        &format!(
            "RAM {} \u{2014} a rewrite of Roblox Account Manager by ic3w0lf22, GPL-3.0. Not affiliated with \
             Roblox Corporation. Stores no passwords or cookies.",
            env!("CARGO_PKG_VERSION")
        ),
    );

    // Hidden windows are skipped by keyboard navigation.
    v.clip = ui::edit_multiline(p, ID_CLIP);
    ui::show(v.clip, false);
    ui::place(v.clip, 0, 0, 0, 0);
    v.first = v.theme;
}

// ---- layout ----------------------------------------------------------------

/// One column of rows: labels on the left, controls on the right. A dry run
/// measures the height without moving anything.
struct Flow {
    hdc: HDC,
    dry: bool,
    x: i32,
    y: i32,
    w: i32,
    label_w: i32,
    row: i32,
    gap: i32,
    line: i32,
    number_w: i32,
    button_w: i32,
}

impl Flow {
    fn put(&self, h: HWND, x: i32, y: i32, w: i32, height: i32) {
        if !self.dry {
            ui::place(h, x, y, w, height);
        }
    }

    fn field_x(&self) -> i32 {
        self.x + self.label_w
    }

    fn field_w(&self) -> i32 {
        self.w - self.label_w
    }

    fn section(&mut self, title: HWND) {
        if self.y > 0 {
            self.y += self.gap * 3 / 2;
        }
        self.put(title, self.x, self.y, self.w, self.line);
        self.y += self.line + self.gap / 2;
    }

    /// Places a row label, vertically centered on the row's control.
    fn label(&self, label: HWND) {
        self.put(label, self.x, self.y + (self.row - self.line) / 2, self.label_w - self.gap, self.line);
    }

    /// A label, a short numeric edit and the hint that follows it.
    fn number(&mut self, label: HWND, edit: HWND, hint: HWND) {
        self.label(label);
        let x = self.field_x();
        self.put(edit, x, self.y, self.number_w, self.row);
        let hint_x = x + self.number_w + self.gap;
        self.put(hint, hint_x, self.y + (self.row - self.line) / 2, self.x + self.w - hint_x, self.line);
        self.y += self.row + self.gap;
    }

    /// Wrapped text across the whole column, as tall as it needs.
    fn text(&mut self, h: HWND) {
        let (_, height) = measure(self.hdc, &ui::get_text(h), self.w);
        self.put(h, self.x, self.y, self.w, height);
        self.y += height;
    }

    fn check(&mut self, h: HWND) {
        self.put(h, self.x, self.y, self.w, self.row);
        self.y += self.row;
    }
}

/// Size of `text` in the font selected into `hdc`, wrapped at `width`.
fn measure(hdc: HDC, text: &str, width: i32) -> (i32, i32) {
    let mut buf: Vec<u16> = text.encode_utf16().collect();
    if buf.is_empty() {
        return (0, 0);
    }
    let mut rc = RECT { left: 0, top: 0, right: width.max(1), bottom: 0 };
    unsafe {
        DrawTextW(hdc, &mut buf, &mut rc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL);
    }
    (rc.right, rc.bottom)
}

/// Width the label column needs for these row labels.
fn label_column(hdc: HDC, labels: &[HWND], gap: i32, max: i32) -> i32 {
    let widest = labels.iter().map(|h| measure(hdc, &ui::get_text(*h), i32::MAX / 2).0).max().unwrap_or(0);
    (widest + 2 * gap).min(max)
}

fn flow_appearance(v: &SettingsView, f: &mut Flow) {
    f.section(v.sections[0]);
    f.label(v.l_theme);
    let field_w = f.field_w().min(f.button_w * 4);
    f.put(v.theme, f.field_x(), f.y, field_w, f.row * 8);
    f.y += f.row + f.gap;
    f.label(v.l_accent);
    let reset_w = f.button_w;
    let edit_w = field_w - reset_w - f.gap;
    f.put(v.accent, f.field_x(), f.y, edit_w, f.row);
    f.put(v.accent_reset, f.field_x() + edit_w + f.gap, f.y, reset_w, f.row);
    f.y += f.row;
    // The validation line doubles as the gap under the row.
    f.put(v.accent_note, f.field_x(), f.y, f.field_w(), f.line);
    f.y += f.line;
    f.check(v.compact);
    f.label(v.l_layout);
    f.put(v.layout, f.field_x(), f.y, field_w, f.row * 6);
    f.y += f.row + f.gap;
    f.text(v.contrast_note);
}

fn flow_launching(v: &SettingsView, f: &mut Flow) {
    f.section(v.sections[1]);
    f.number(v.l_concurrency, v.concurrency, v.h_concurrency);
    f.y -= f.gap / 2;
    f.text(v.concurrency_note);
    f.y += f.gap;
    f.number(v.l_spacing, v.spacing, v.h_spacing);
    f.number(v.l_timeout, v.timeout, v.h_timeout);
    f.y -= f.gap / 2;
    f.check(v.include_main);
    f.text(v.main_note);
    f.check(v.direct);
    f.check(v.multi);
    f.text(v.multi_note);
}

fn flow_security(v: &SettingsView, f: &mut Flow, configured: bool, unlocked: bool, master: bool) {
    f.section(v.sections[2]);
    f.text(v.vault_status);
    f.y += f.gap / 2;
    let mut row: Vec<HWND> = Vec::new();
    if !configured {
        row.push(v.vault_setup);
    } else {
        row.push(if unlocked { v.vault_lock } else { v.vault_unlock });
        if master {
            row.push(v.vault_change);
        }
        row.push(v.vault_remove);
    }
    let bw = f.button_w * 5 / 2;
    let mut x = f.x;
    for &h in &row {
        if x + bw > f.x + f.w {
            x = f.x;
            f.y += f.row + f.gap / 2;
        }
        f.put(h, x, f.y, bw, f.row);
        x += bw + f.gap;
    }
    f.y += f.row + f.gap;
    if configured {
        f.check(v.vault_hello);
        if master {
            f.number(v.l_autolock, v.autolock, v.h_autolock);
        }
    }
}

fn flow_oauth(v: &SettingsView, f: &mut Flow) {
    f.section(v.sections[3]);
    f.text(v.oauth_note);
    f.y += f.gap;
    f.label(v.l_client_id);
    f.put(v.client_id, f.field_x(), f.y, f.field_w(), f.row);
    f.y += f.row + f.gap;
    f.number(v.l_port, v.port, v.h_port);
    f.label(v.l_redirect);
    let copy_w = f.button_w;
    f.put(v.redirect, f.field_x(), f.y, f.field_w() - copy_w - f.gap, f.row);
    f.put(v.copy_url, f.x + f.w - copy_w, f.y, copy_w, f.row);
    f.y += f.row;
}

fn flow_data(v: &SettingsView, f: &mut Flow) {
    f.section(v.sections[4]);
    f.put(v.data_path, f.x, f.y, f.w, f.line);
    f.y += f.line + f.gap / 2;
    let button_w = ((f.w - f.gap) / 2).min(f.number_w * 2 + f.gap);
    f.put(v.open_dir, f.x, f.y, button_w, f.row);
    f.put(v.copy_diag, f.x + button_w + f.gap, f.y, button_w, f.row);
    f.y += f.row + f.gap;
    f.put(v.export_btn, f.x, f.y, button_w * 2 + f.gap, f.row);
    f.y += f.row + f.gap;
    f.text(v.about);
}

pub fn layout(app: &mut App) {
    let panel = app.panels[3];
    let rc = ui::client_rect(panel);
    let m = &app.m;
    let v = &app.settings;
    let gap = m.gap();
    let hdc = unsafe { GetDC(Some(panel)) };
    let old_font = unsafe { SelectObject(hdc, m.font.into()) };
    let (_, line) = measure(hdc, "Ag", i32::MAX / 2);
    let flow = |dry: bool, x: i32, w: i32, label_w: i32| Flow {
        hdc,
        dry,
        x,
        y: 0,
        w,
        label_w,
        row: m.row(),
        gap,
        line: line.max(m.px(15)) + m.px(2),
        number_w: m.px(64),
        button_w: m.px(60),
    };
    let configured = app.vault_configured();
    let unlocked = app.is_vault_unlocked();
    let master = app.vault.mode() == Some(ram_core::vault::VaultMode::MasterPassword);
    let all = |f: &mut Flow| {
        flow_appearance(v, f);
        flow_launching(v, f);
        flow_security(v, f, configured, unlocked, master);
        flow_oauth(v, f);
        flow_data(v, f);
    };

    let single_w = (rc.right - 2 * gap).min(m.px(640));
    let mut probe = flow(true, gap, single_w, m.px(220));
    all(&mut probe);
    if probe.y <= rc.bottom {
        let mut f = flow(false, gap, single_w, m.px(220));
        all(&mut f);
    } else {
        // Too tall for one column: appearance and launching on the left,
        // security, sign-in and data on the right.
        let between = 3 * gap;
        let column_w = ((rc.right - 2 * gap - between) / 2).min(m.px(640));
        let left_labels = [v.l_theme, v.l_accent, v.l_layout, v.l_concurrency, v.l_spacing, v.l_timeout];
        let right_labels = [v.l_autolock, v.l_client_id, v.l_port, v.l_redirect];
        let mut left = flow(false, gap, column_w, label_column(hdc, &left_labels, gap, m.px(220)));
        flow_appearance(v, &mut left);
        flow_launching(v, &mut left);
        let right_x = gap + column_w + between;
        let mut right = flow(false, right_x, column_w, label_column(hdc, &right_labels, gap, m.px(220)));
        flow_security(v, &mut right, configured, unlocked, master);
        flow_oauth(v, &mut right);
        flow_data(v, &mut right);
    }
    unsafe {
        SelectObject(hdc, old_font);
        ReleaseDC(Some(panel), hdc);
    }
}

// ---- data ------------------------------------------------------------------

fn accent_text(accent: Option<u32>) -> String {
    accent.map(|c| format!("#{:06X}", c & 0xFF_FFFF)).unwrap_or_default()
}

/// Parses the accent field: `Ok(None)` when empty, `Err` when it is not
/// `#RRGGBB`.
fn parse_accent(text: &str) -> Result<Option<u32>, ()> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let digits = t.strip_prefix('#').unwrap_or(t);
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        crate::theme::parse_hex(digits).map(Some).ok_or(())
    } else {
        Err(())
    }
}

/// The edit, range and stored value of a numeric setting.
fn number_field(app: &App, id: u16) -> Option<(HWND, u32, u32, u32)> {
    let (v, s) = (&app.settings, &app.store.settings);
    match id {
        ID_CONCURRENCY => Some((v.concurrency, 1, 4, s.launch_concurrency)),
        ID_SPACING => Some((v.spacing, 0, 120, s.launch_spacing_secs)),
        ID_TIMEOUT => Some((v.timeout, 20, 600, s.launch_timeout_secs)),
        ID_PORT => Some((v.port, 1024, 65535, s.oauth_port as u32)),
        _ => None,
    }
}

/// Fills the controls from the stored settings. A field that has keyboard
/// focus keeps what the user is typing.
pub fn refresh(app: &mut App) {
    let focus = ui::focused();
    app.settings.loading = true;
    let (v, s) = (&app.settings, &app.store.settings);
    let theme_index = ThemeKind::all().iter().position(|k| *k == s.theme).unwrap_or(0);
    ui::combo_select(v.theme, theme_index as i32);
    if focus != v.accent {
        ui::set_text_if_changed(v.accent, &accent_text(s.accent));
        ui::set_text_if_changed(v.accent_note, "");
    }
    ui::set_checked(v.compact, s.compact);
    ui::set_checked(v.include_main, s.include_main_in_batch);
    for id in [ID_CONCURRENCY, ID_SPACING, ID_TIMEOUT, ID_PORT] {
        if let Some((edit, min, max, value)) = number_field(app, id) {
            if focus != edit {
                ui::set_text_if_changed(edit, &value.clamp(min, max).to_string());
            }
        }
    }
    if focus != v.client_id {
        ui::set_text_if_changed(v.client_id, &s.oauth_client_id);
    }
    ui::set_text_if_changed(v.redirect, &oauth::redirect_uri(s.oauth_port));

    let layout_index = i32::from(s.layout == ram_core::model::LayoutMode::Classic);
    ui::combo_select(v.layout, layout_index);
    ui::set_checked(v.direct, s.direct_launch);
    ui::set_checked(v.multi, s.multi_instance);

    let configured = app.vault_configured();
    let unlocked = app.is_vault_unlocked();
    let master = app.vault.mode() == Some(ram_core::vault::VaultMode::MasterPassword);
    let status = app.vault_status_text();
    let hello = app.vault.require_hello_reveal();
    let autolock_min = app.vault.auto_lock_secs() / 60;
    let v = &app.settings;
    ui::set_text_if_changed(v.vault_status, &status);
    ui::show(v.vault_setup, !configured);
    ui::show(v.vault_unlock, configured && !unlocked);
    ui::show(v.vault_lock, configured && unlocked);
    ui::show(v.vault_change, configured && master);
    ui::show(v.vault_remove, configured);
    ui::show(v.vault_hello, configured);
    ui::show(v.l_autolock, configured && master);
    ui::show(v.autolock, configured && master);
    ui::show(v.h_autolock, configured && master);
    ui::set_checked(v.vault_hello, hello);
    if focus != v.autolock {
        ui::set_text_if_changed(v.autolock, &autolock_min.to_string());
    }
    app.settings.loading = false;
}

/// Restyles the window after a theme, accent or density change.
fn apply_look(app: &mut App) {
    app.settings.loading = true;
    app.apply_theme();
    app.settings.loading = false;
    app.mark_dirty();
}

fn set_accent(app: &mut App, accent: Option<u32>) {
    if app.store.settings.accent != accent {
        app.store.settings.accent = accent;
        apply_look(app);
    }
}

fn accent_changed(app: &mut App) {
    match parse_accent(&ui::get_text(app.settings.accent)) {
        Ok(accent) => {
            ui::set_text_if_changed(app.settings.accent_note, "");
            set_accent(app, accent);
        }
        Err(()) => ui::set_text_if_changed(app.settings.accent_note, ACCENT_INVALID),
    }
}

fn number_changed(app: &mut App, id: u16) {
    let Some((edit, min, max, stored)) = number_field(app, id) else { return };
    let Ok(typed) = ui::get_text(edit).trim().parse::<u32>() else { return };
    let value = typed.clamp(min, max);
    if value == stored {
        return;
    }
    let s = &mut app.store.settings;
    match id {
        ID_CONCURRENCY => s.launch_concurrency = value,
        ID_SPACING => s.launch_spacing_secs = value,
        ID_TIMEOUT => s.launch_timeout_secs = value,
        ID_PORT => {
            s.oauth_port = value as u16;
            ui::set_text_if_changed(app.settings.redirect, &oauth::redirect_uri(value as u16));
        }
        _ => return,
    }
    app.mark_dirty();
}

pub fn command(app: &mut App, id: u16, code: u16, _ctl: HWND) {
    if app.settings.loading {
        return;
    }
    match (id, code) {
        (ID_THEME, CBN_SELCHANGE_CODE) => {
            let picked = ui::combo_selected(app.settings.theme).and_then(|i| ThemeKind::all().get(i)).copied();
            if let Some(kind) = picked.filter(|kind| *kind != app.store.settings.theme) {
                app.store.settings.theme = kind;
                apply_look(app);
            }
        }
        (ID_ACCENT, EN_CHANGE_CODE) => accent_changed(app),
        (ID_ACCENT_RESET, CLICKED) => {
            app.settings.loading = true;
            ui::set_text(app.settings.accent, "");
            ui::set_text_if_changed(app.settings.accent_note, "");
            app.settings.loading = false;
            set_accent(app, None);
        }
        (ID_COMPACT, CLICKED) => {
            let on = ui::is_checked(app.settings.compact);
            if app.store.settings.compact != on {
                app.store.settings.compact = on;
                apply_look(app);
            }
        }
        (ID_CONCURRENCY | ID_SPACING | ID_TIMEOUT | ID_PORT, EN_CHANGE_CODE) => number_changed(app, id),
        // Leaving a field shows the value that is actually in effect.
        (ID_ACCENT | ID_CONCURRENCY | ID_SPACING | ID_TIMEOUT | ID_PORT, EN_KILLFOCUS_CODE) => refresh(app),
        (ID_INCLUDE_MAIN, CLICKED) => {
            app.store.settings.include_main_in_batch = ui::is_checked(app.settings.include_main);
            app.mark_dirty();
        }
        (ID_CLIENT_ID, EN_CHANGE_CODE) => {
            let text = ui::get_text(app.settings.client_id).trim().to_string();
            if app.store.settings.oauth_client_id != text {
                app.store.settings.oauth_client_id = text;
                app.mark_dirty();
            }
        }
        (ID_COPY_URL, CLICKED) => {
            let url = oauth::redirect_uri(app.store.settings.oauth_port);
            copy_text(app, &url, "Redirect URL");
        }
        (ID_OPEN_DIR, CLICKED) => open_data_dir(app),
        (ID_COPY_DIAG, CLICKED) => {
            let report = diagnostics(app);
            copy_text(app, &report, "Diagnostics");
        }
        (ID_LAYOUT, CBN_SELCHANGE_CODE) => {
            let classic = ui::combo_selected(app.settings.layout) == Some(1);
            let want = if classic { ram_core::model::LayoutMode::Classic } else { ram_core::model::LayoutMode::Modern };
            if app.store.settings.layout != want {
                app.store.settings.layout = want;
                app.mark_dirty();
                crate::accounts::refresh(app);
            }
        }
        (ID_DIRECT, CLICKED) => {
            app.store.settings.direct_launch = ui::is_checked(app.settings.direct);
            app.mark_dirty();
        }
        (ID_MULTI, CLICKED) => {
            app.store.settings.multi_instance = ui::is_checked(app.settings.multi);
            app.mark_dirty();
            app.apply_multi_instance();
            app.set_status(if app.store.settings.multi_instance {
                "Multi-instance on: several Roblox clients can run at once."
            } else {
                "Multi-instance off. Close Roblox before changing this to take effect."
            });
        }
        (ID_VAULT_SETUP, CLICKED) => app.setup_vault_flow(),
        (ID_VAULT_UNLOCK, CLICKED) => {
            app.ensure_vault_unlocked();
            refresh(app);
        }
        (ID_VAULT_LOCK, CLICKED) => app.lock_vault(),
        (ID_VAULT_CHANGE, CLICKED) => app.change_master_password_flow(),
        (ID_VAULT_REMOVE, CLICKED) => app.remove_vault_flow(),
        (ID_VAULT_HELLO, CLICKED) => {
            let on = ui::is_checked(app.settings.vault_hello);
            app.set_require_hello(on);
        }
        (ID_VAULT_AUTOLOCK, EN_CHANGE_CODE) => {
            if let Ok(min) = ui::get_text(app.settings.autolock).trim().parse::<u32>() {
                app.set_auto_lock_minutes(min.min(1440));
            }
        }
        (ID_EXPORT, CLICKED) => app.export_accounts_flow(),
        _ => {}
    }
}

// ---- actions ---------------------------------------------------------------

/// Puts `text` on the clipboard through the hidden edit control and reports
/// the outcome in the status bar.
fn copy_text(app: &mut App, text: &str, what: &str) {
    let clip = app.settings.clip;
    ui::set_text(clip, text);
    let before = unsafe { GetClipboardSequenceNumber() };
    ui::send(clip, EM_SETSEL, 0, -1);
    ui::send(clip, WM_COPY, 0, 0);
    let copied = unsafe { GetClipboardSequenceNumber() } != before;
    ui::set_text(clip, "");
    app.set_status(&if copied {
        format!("{what} copied to the clipboard.")
    } else {
        format!("{what} could not be copied: the clipboard is in use by another program.")
    });
}

fn open_data_dir(app: &mut App) {
    let dir = ui::wide(&app.data_dir.display().to_string());
    let result =
        unsafe { ShellExecuteW(Some(app.hwnd), w!("open"), PCWSTR(dir.as_ptr()), None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success as a value above 32.
    if result.0 as usize <= 32 {
        app.set_status("The data folder could not be opened.");
    }
}

/// The Windows build number, such as "26100".
fn windows_build() -> Option<String> {
    let mut buf = [0u16; 32];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    let build = ui::from_wide(&buf);
    (read.is_ok() && !build.is_empty()).then_some(build)
}

/// A plain-text report for bug reports: counts, non-sensitive settings and
/// the end of the log. It contains no account names, client ID or paths.
fn diagnostics(app: &App) -> String {
    let (store, s) = (&app.store, &app.store.settings);
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    let mut lines = vec![format!("RAM {}", env!("CARGO_PKG_VERSION"))];
    if let Some(build) = windows_build() {
        lines.push(format!("Windows build {build}"));
    }
    lines.push(String::new());
    lines.push(format!("Accounts: {}", store.accounts.len()));
    lines.push(format!("Destinations: {}", store.destinations.len()));
    lines.push(format!("Games: {}", store.games.len()));
    lines.push(format!("Running clients: {}", app.clients.processes().len()));
    lines.push(format!("Active overrides: {}", store.applied.len()));
    lines.push(String::new());
    lines.push(format!("Theme: {:?} (custom accent: {})", s.theme, yes_no(s.accent.is_some())));
    lines.push(format!("Compact density: {}", yes_no(s.compact)));
    lines.push(format!(
        "Launching: {} in flight, {} s apart, {} s timeout, Main in batches: {}",
        s.launch_concurrency,
        s.launch_spacing_secs,
        s.launch_timeout_secs,
        yes_no(s.include_main_in_batch)
    ));
    lines.push(format!("OAuth client ID set: {}", yes_no(!s.oauth_client_id.trim().is_empty())));
    lines.push(format!("Performance preset: {}", store.perf.preset.label()));
    lines.push(String::new());

    // Account names never belong in a report, even when a log line has one.
    let mut names: Vec<&str> = store
        .accounts
        .iter()
        .flat_map(|a| [a.username.trim(), a.alias.trim()])
        .filter(|n| n.chars().count() >= 3)
        .collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    match std::fs::read(app.log.path()) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let all: Vec<&str> = text.lines().collect();
            let tail = &all[all.len().saturating_sub(LOG_TAIL_LINES)..];
            lines.push(format!("Log (last {} lines):", tail.len()));
            for line in tail {
                let mut line = redact(line);
                for name in &names {
                    if line.contains(name) {
                        line = line.replace(name, "[account]");
                    }
                }
                lines.push(line);
            }
        }
        Err(_) => lines.push("Log: none yet.".to_string()),
    }
    lines.join("\r\n")
}
