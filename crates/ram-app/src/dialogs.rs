//! Modal dialogs built from the same themed controls as the main window.

use crate::app::{set_fonts, try_app, LABEL, LABEL_MUTED};
use crate::theme;
use crate::ui::{self, wide, Metrics};
use std::cell::Cell;
use std::ffi::c_void;
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const DIALOG_CLASS: PCWSTR = w!("RAMDialog");
const ID_OK: u16 = 1;
const ID_CANCEL: u16 = 2;
const ID_ERROR: u16 = 90;
const ID_FIELD: u16 = 100;
const ID_CHECKLIST: u16 = 200;

thread_local! {
    static RESULT: Cell<u16> = const { Cell::new(0) };
}

pub fn register() {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(dialog_proc),
            hInstance: ui::hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: DIALOG_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = ui::loword(wparam.0);
            if id == ID_OK || id == ID_CANCEL {
                RESULT.with(|r| r.set(id));
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            RESULT.with(|r| r.set(ID_CANCEL));
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            if let Some(app) = try_app() {
                let rc = ui::client_rect(hwnd);
                FillRect(HDC(wparam.0 as *mut _), &rc, app.brushes.window);
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => match try_app().and_then(|app| app.theme_message(msg, wparam, lparam)) {
            Some(r) => r,
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        },
    }
}

/// Runs a modal dialog. `build` creates the controls; `accept` runs when the
/// user confirms and returns false to keep the dialog open (validation).
/// Returns true when the dialog was accepted.
fn modal(
    owner: HWND,
    title: &str,
    width_dip: i32,
    height_dip: i32,
    ok_label: &str,
    build: &mut dyn FnMut(HWND, &Metrics, i32, i32),
    accept: &mut dyn FnMut(HWND) -> bool,
) -> bool {
    let Some(app) = try_app() else { return false };
    let m = Metrics::new(app.m.dpi, app.m.compact);
    let (cw, ch) = (m.px(width_dip), m.px(height_dip));
    unsafe {
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let ex = WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT;
        let mut rc = RECT { left: 0, top: 0, right: cw, bottom: ch };
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut rc, style, false, ex, m.dpi);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut orc = RECT::default();
        let _ = GetWindowRect(owner, &mut orc);
        let x = orc.left + ((orc.right - orc.left) - ww) / 2;
        let y = orc.top + ((orc.bottom - orc.top) - wh) / 2;
        let Ok(dlg) = CreateWindowExW(
            ex,
            DIALOG_CLASS,
            PCWSTR(wide(title).as_ptr()),
            style,
            x,
            y.max(0),
            ww,
            wh,
            Some(owner),
            None,
            Some(ui::hinstance()),
            None,
        ) else {
            return false;
        };

        let (gap, row) = (m.gap(), m.row());
        let button_w = m.px(96);
        let content_h = ch - row - 2 * gap;
        build(dlg, &m, cw, content_h);
        let error = ui::label(dlg, ID_ERROR, "");
        ui::place(error, gap, ch - row - gap, cw - 2 * button_w - 4 * gap, row);
        let ok = ui::button(dlg, ID_OK, ok_label);
        let cancel = ui::button(dlg, ID_CANCEL, "Cancel");
        ui::place(ok, cw - 2 * button_w - 2 * gap, ch - row - gap, button_w, row);
        ui::place(cancel, cw - button_w - gap, ch - row - gap, button_w, row);
        set_fonts(dlg, app.m.font);
        theme::apply_to_frame(dlg, &app.palette);
        theme::apply_to_tree(dlg, &app.palette);

        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(dlg, SW_SHOW);
        if let Ok(first) = GetNextDlgTabItem(dlg, None, false) {
            ui::focus(first);
        }
        RESULT.with(|r| r.set(0));
        let mut accepted = false;
        let mut msg = MSG::default();
        loop {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 <= 0 {
                // WM_QUIT: pass it on to the main loop.
                if got.0 == 0 {
                    PostQuitMessage(msg.wParam.0 as i32);
                }
                break;
            }
            if !IsDialogMessageW(dlg, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            match RESULT.with(|r| r.replace(0)) {
                ID_OK => {
                    if accept(dlg) {
                        accepted = true;
                        break;
                    }
                }
                ID_CANCEL => break,
                _ => {}
            }
        }
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
        let _ = DestroyWindow(dlg);
        accepted
    }
}

fn show_error(dlg: HWND, text: &str) {
    unsafe {
        if let Ok(h) = GetDlgItem(Some(dlg), ID_ERROR as i32) {
            ui::set_text(h, text);
        }
    }
}

pub struct Field<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub cue: &'a str,
    pub multiline: bool,
}

impl<'a> Field<'a> {
    pub fn line(label: &'a str, value: &'a str, cue: &'a str) -> Self {
        Field { label, value, cue, multiline: false }
    }
}

/// Asks for one or more text values. `validate` returns an error message to
/// show inline; the dialog stays open until the input is valid or cancelled.
pub fn prompt(
    owner: HWND,
    title: &str,
    intro: &str,
    ok_label: &str,
    fields: &[Field],
    validate: &dyn Fn(&[String]) -> Result<(), String>,
) -> Option<Vec<String>> {
    let lines: i32 = fields.iter().map(|f| if f.multiline { 5 } else { 2 }).sum();
    let intro_rows = if intro.is_empty() { 0 } else { 2 };
    let height = 24 + (lines + intro_rows) * 30 + 48;
    let mut edits: Vec<HWND> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    let ok = modal(
        owner,
        title,
        460,
        height,
        ok_label,
        &mut |dlg, m, cw, _| {
            let (gap, row) = (m.gap(), m.row());
            let mut y = gap;
            if !intro.is_empty() {
                let p = ui::paragraph(dlg, LABEL_MUTED, intro);
                ui::place(p, gap, y, cw - 2 * gap, row * 2);
                y += row * 2 + gap / 2;
            }
            for (i, f) in fields.iter().enumerate() {
                let l = ui::label(dlg, LABEL, f.label);
                ui::place(l, gap, y, cw - 2 * gap, row - m.px(6));
                y += row - m.px(4);
                let id = ID_FIELD + i as u16;
                let (e, h) = if f.multiline {
                    (ui::edit_multiline(dlg, id), row * 3)
                } else {
                    (ui::edit(dlg, id), row)
                };
                ui::place(e, gap, y, cw - 2 * gap, h);
                ui::set_text(e, f.value);
                if !f.cue.is_empty() {
                    ui::set_cue(e, f.cue);
                }
                edits.push(e);
                y += h + gap;
            }
        },
        &mut |dlg| {
            let current: Vec<String> = (0..fields.len())
                .map(|i| unsafe {
                    GetDlgItem(Some(dlg), (ID_FIELD + i as u16) as i32).map(ui::get_text).unwrap_or_default()
                })
                .collect();
            match validate(&current) {
                Ok(()) => {
                    values = current;
                    true
                }
                Err(e) => {
                    show_error(dlg, &e);
                    false
                }
            }
        },
    );
    let _ = edits;
    ok.then_some(values)
}

pub struct CheckRow {
    pub label: String,
    pub action: String,
    pub detail: String,
    pub checked: bool,
    /// Rows that cannot be applied are shown but cannot be ticked.
    pub enabled: bool,
}

/// Shows a preview list with a check box per row and returns the final tick
/// state of every row, or `None` when cancelled.
pub fn checklist(owner: HWND, title: &str, intro: &str, ok_label: &str, rows: &[CheckRow]) -> Option<Vec<bool>> {
    let mut result: Vec<bool> = Vec::new();
    let ok = modal(
        owner,
        title,
        760,
        520,
        ok_label,
        &mut |dlg, m, cw, content_h| {
            let (gap, row) = (m.gap(), m.row());
            let intro_h = row * 3;
            let p = ui::paragraph(dlg, LABEL_MUTED, intro);
            ui::place(p, gap, gap, cw - 2 * gap, intro_h);
            let list = ui::listview_plain(dlg, ID_CHECKLIST);
            ui::send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, (LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize);
            ui::place(list, gap, gap + intro_h + gap, cw - 2 * gap, content_h - intro_h - 2 * gap);
            ui::list_add_column(list, 0, "Item", m.px(280));
            ui::list_add_column(list, 1, "Action", m.px(110));
            ui::list_add_column(list, 2, "Details", m.px(330));
            for (i, r) in rows.iter().enumerate() {
                ui::list_insert_row(list, i, &[&r.label, &r.action, &r.detail]);
                ui::list_set_check(list, i, r.checked && r.enabled);
            }
        },
        &mut |dlg| {
            let list = unsafe { GetDlgItem(Some(dlg), ID_CHECKLIST as i32).unwrap_or_default() };
            result = rows.iter().enumerate().map(|(i, r)| r.enabled && ui::list_get_check(list, i)).collect();
            true
        },
    );
    ok.then_some(result)
}

/// Lets the user pick one row from a short list; returns its index.
pub fn choose(owner: HWND, title: &str, intro: &str, ok_label: &str, options: &[String]) -> Option<usize> {
    let mut picked = None;
    let ok = modal(
        owner,
        title,
        420,
        170,
        ok_label,
        &mut |dlg, m, cw, _| {
            let (gap, row) = (m.gap(), m.row());
            let p = ui::paragraph(dlg, LABEL_MUTED, intro);
            ui::place(p, gap, gap, cw - 2 * gap, row * 2);
            let c = ui::combo(dlg, ID_FIELD);
            ui::place(c, gap, gap + row * 2 + gap, cw - 2 * gap, row * 12);
            for o in options {
                ui::combo_add(c, o);
            }
            ui::combo_select(c, 0);
        },
        &mut |dlg| {
            picked = unsafe { GetDlgItem(Some(dlg), ID_FIELD as i32).ok().and_then(ui::combo_selected) };
            picked.is_some()
        },
    );
    if ok {
        picked
    } else {
        None
    }
}

/// Asks for a single secret value in a masked field (master password,
/// pasted cookie). `validate` returns an error to show inline.
pub fn prompt_secret(
    owner: HWND,
    title: &str,
    intro: &str,
    label: &str,
    ok_label: &str,
    validate: &dyn Fn(&str) -> Result<(), String>,
) -> Option<String> {
    let mut value = None;
    let ok = modal(
        owner,
        title,
        460,
        190,
        ok_label,
        &mut |dlg, m, cw, _| {
            let (gap, row) = (m.gap(), m.row());
            let mut y = gap;
            if !intro.is_empty() {
                let p = ui::paragraph(dlg, LABEL_MUTED, intro);
                ui::place(p, gap, y, cw - 2 * gap, row * 2);
                y += row * 2 + gap / 2;
            }
            let l = ui::label(dlg, LABEL, label);
            ui::place(l, gap, y, cw - 2 * gap, row - m.px(4));
            y += row - m.px(2);
            let e = ui::edit_password(dlg, ID_FIELD);
            ui::place(e, gap, y, cw - 2 * gap, row);
        },
        &mut |dlg| {
            let text = unsafe { GetDlgItem(Some(dlg), ID_FIELD as i32).map(ui::get_text).unwrap_or_default() };
            match validate(&text) {
                Ok(()) => {
                    value = Some(text);
                    true
                }
                Err(e) => {
                    show_error(dlg, &e);
                    false
                }
            }
        },
    );
    ok.then_some(value).flatten()
}

/// Shows a secret in a read-only field for the user to read. The primary
/// button copies it to the clipboard. Used behind a Windows Hello or
/// password gate.
pub fn show_secret(owner: HWND, title: &str, intro: &str, text: &str) {
    let text = text.to_string();
    modal(
        owner,
        title,
        540,
        230,
        "Copy",
        &mut |dlg, m, cw, _| {
            let (gap, row) = (m.gap(), m.row());
            let p = ui::paragraph(dlg, LABEL_MUTED, intro);
            ui::place(p, gap, gap, cw - 2 * gap, row * 2);
            let e = ui::edit_multiline(dlg, ID_FIELD);
            ui::place(e, gap, gap + row * 2 + gap, cw - 2 * gap, row * 3);
            ui::set_text(e, &text);
            // EM_SETREADONLY: show the secret without letting it be edited.
            ui::send(e, 0x00CF, 1, 0);
        },
        &mut |dlg| {
            let _ = ui::copy_to_clipboard(dlg, &text);
            true
        },
    );
}

/// The system folder picker.
pub fn pick_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST).ok()?;
        let _ = dialog.SetTitle(PCWSTR(wide(title).as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const c_void));
        text.map(PathBuf::from)
    }
}
