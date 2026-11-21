//! Thin helpers over Win32 controls. Everything UI-related runs on the one
//! GUI thread.

use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

pub fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

pub fn post(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam));
    }
}

pub fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

pub fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}

/// Sets the text only when it differs, so an edit keeps its caret and a
/// static does not flicker.
pub fn set_text_if_changed(hwnd: HWND, text: &str) {
    if get_text(hwnd) != text {
        set_text(hwnd, text);
    }
}

pub fn show(hwnd: HWND, visible: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

pub fn enable(hwnd: HWND, enabled: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, enabled);
    }
}

pub fn focus(hwnd: HWND) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
}

pub fn focused() -> HWND {
    unsafe { GetFocus() }
}

pub fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w.max(0), h.max(0), SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn client_rect(hwnd: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    rc
}

pub fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

pub fn dpi_of(hwnd: HWND) -> u32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 {
        96
    } else {
        d
    }
}

/// Fonts and metrics for the current DPI and density.
pub struct Metrics {
    pub dpi: u32,
    pub compact: bool,
    pub font: HFONT,
    pub font_bold: HFONT,
}

impl Metrics {
    pub fn new(dpi: u32, compact: bool) -> Self {
        let (font, font_bold) = make_fonts(dpi);
        Metrics { dpi, compact, font, font_bold }
    }

    /// Converts device-independent pixels to physical pixels.
    pub fn px(&self, dip: i32) -> i32 {
        (dip as i64 * self.dpi as i64 / 96) as i32
    }

    /// Height of an input row.
    pub fn row(&self) -> i32 {
        self.px(if self.compact { 24 } else { 28 })
    }

    /// Gap between controls.
    pub fn gap(&self) -> i32 {
        self.px(if self.compact { 6 } else { 8 })
    }

    pub fn list_row(&self) -> i32 {
        self.px(if self.compact { 22 } else { 28 })
    }

    pub fn release(&mut self) {
        unsafe {
            let _ = DeleteObject(self.font.into());
            let _ = DeleteObject(self.font_bold.into());
        }
    }
}

fn make_fonts(dpi: u32) -> (HFONT, HFONT) {
    unsafe {
        let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let ok = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            ncm.cbSize,
            Some(&mut ncm as *mut _ as *mut c_void),
            0,
            dpi,
        )
        .is_ok();
        let mut lf = if ok {
            ncm.lfMessageFont
        } else {
            let mut lf = LOGFONTW { lfHeight: -(12 * dpi as i32 / 96), ..Default::default() };
            for (i, c) in "Segoe UI".encode_utf16().enumerate() {
                lf.lfFaceName[i] = c;
            }
            lf
        };
        let regular = CreateFontIndirectW(&lf);
        lf.lfWeight = 600;
        let bold = CreateFontIndirectW(&lf);
        (regular, bold)
    }
}

fn create(
    class: PCWSTR,
    text: &str,
    style: u32,
    ex: u32,
    parent: HWND,
    id: u16,
) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(ex),
            class,
            PCWSTR(wide(text).as_ptr()),
            WINDOW_STYLE(style | WS_CHILD.0 | WS_VISIBLE.0),
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as usize as *mut c_void)),
            Some(hinstance()),
            None,
        )
        .unwrap_or_default()
    }
}

pub fn label(parent: HWND, id: u16, text: &str) -> HWND {
    create(w!("STATIC"), text, 0x80 | 0x4000, 0, parent, id)
}

/// A static that wraps its text over several lines.
pub fn paragraph(parent: HWND, id: u16, text: &str) -> HWND {
    create(w!("STATIC"), text, 0x80 | 0x2000, 0, parent, id)
}

pub fn edit(parent: HWND, id: u16) -> HWND {
    create(w!("EDIT"), "", WS_TABSTOP.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32, 0, parent, id)
}

/// A single-line edit that masks its characters, for passwords.
pub fn edit_password(parent: HWND, id: u16) -> HWND {
    create(w!("EDIT"), "", WS_TABSTOP.0 | WS_BORDER.0 | (ES_AUTOHSCROLL | ES_PASSWORD) as u32, 0, parent, id)
}

/// Copies text to the clipboard through a throwaway edit control and
/// `WM_COPY`, so no extra clipboard features are needed. Returns true on
/// success.
pub fn copy_to_clipboard(parent: HWND, text: &str) -> bool {
    unsafe {
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(wide(text).as_ptr()),
            WS_CHILD | WINDOW_STYLE(ES_MULTILINE as u32),
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(hinstance()),
            None,
        )
        .unwrap_or_default();
        if edit.is_invalid() {
            return false;
        }
        send(edit, EM_SETSEL, 0, -1);
        let ok = send(edit, WM_COPY, 0, 0);
        let _ = DestroyWindow(edit);
        ok != 0
    }
}

pub fn edit_multiline(parent: HWND, id: u16) -> HWND {
    create(
        w!("EDIT"),
        "",
        WS_TABSTOP.0 | WS_BORDER.0 | WS_VSCROLL.0 | (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32,
        0,
        parent,
        id,
    )
}

pub fn button(parent: HWND, id: u16, text: &str) -> HWND {
    create(w!("BUTTON"), text, WS_TABSTOP.0 | BS_OWNERDRAW as u32, 0, parent, id)
}

pub fn checkbox(parent: HWND, id: u16, text: &str) -> HWND {
    create(w!("BUTTON"), text, WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32, 0, parent, id)
}

/// A drop-down list the user picks from.
pub fn combo(parent: HWND, id: u16) -> HWND {
    create(w!("COMBOBOX"), "", WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32 | CBS_HASSTRINGS as u32, 0, parent, id)
}

/// A drop-down the user can also type into.
pub fn combo_editable(parent: HWND, id: u16) -> HWND {
    create(
        w!("COMBOBOX"),
        "",
        WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWN as u32 | CBS_HASSTRINGS as u32 | CBS_AUTOHSCROLL as u32,
        0,
        parent,
        id,
    )
}

pub fn combo_clear(hwnd: HWND) {
    send(hwnd, CB_RESETCONTENT, 0, 0);
}

pub fn combo_add(hwnd: HWND, text: &str) {
    let t = wide(text);
    send(hwnd, CB_ADDSTRING, 0, t.as_ptr() as isize);
}

pub fn combo_select(hwnd: HWND, index: i32) {
    send(hwnd, CB_SETCURSEL, index as usize, 0);
}

pub fn combo_selected(hwnd: HWND) -> Option<usize> {
    let i = send(hwnd, CB_GETCURSEL, 0, 0);
    (i >= 0).then_some(i as usize)
}

pub fn is_checked(hwnd: HWND) -> bool {
    send(hwnd, BM_GETCHECK, 0, 0) == 1
}

pub fn set_checked(hwnd: HWND, on: bool) {
    send(hwnd, BM_SETCHECK, on as usize, 0);
}

/// Placeholder text for an empty edit control.
pub fn set_cue(hwnd: HWND, text: &str) {
    let t = wide(text);
    send(hwnd, EM_SETCUEBANNER, 1, t.as_ptr() as isize);
}

pub const LIST_STYLE_SINGLE: u32 = LVS_SINGLESEL;

/// A virtual (owner-data) report list. Rows are supplied on demand through
/// LVN_GETDISPINFO, so the control holds no per-row memory.
pub fn listview(parent: HWND, id: u16, extra_style: u32) -> HWND {
    let h = create(
        w!("SysListView32"),
        "",
        WS_TABSTOP.0 | LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | extra_style,
        0,
        parent,
        id,
    );
    send(
        h,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        0,
        (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_HEADERDRAGDROP | LVS_EX_LABELTIP) as isize,
    );
    h
}

/// A report list that stores its own rows; used for short preview lists.
pub fn listview_plain(parent: HWND, id: u16) -> HWND {
    create(w!("SysListView32"), "", WS_TABSTOP.0 | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SINGLESEL, 0, parent, id)
}

pub fn list_insert_row(hwnd: HWND, index: usize, cells: &[&str]) {
    for (col, text) in cells.iter().enumerate() {
        let mut t = wide(text);
        let item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: index as i32,
            iSubItem: col as i32,
            pszText: windows::core::PWSTR(t.as_mut_ptr()),
            ..Default::default()
        };
        let msg = if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMW };
        send(hwnd, msg, 0, &item as *const _ as isize);
    }
}

pub fn list_set_check(hwnd: HWND, index: usize, checked: bool) {
    // State image 1 is unchecked, 2 is checked.
    let item = LVITEMW {
        stateMask: LVIS_STATEIMAGEMASK,
        state: LIST_VIEW_ITEM_STATE_FLAGS(((checked as u32) + 1) << 12),
        ..Default::default()
    };
    send(hwnd, LVM_SETITEMSTATE, index, &item as *const _ as isize);
}

pub fn list_get_check(hwnd: HWND, index: usize) -> bool {
    let state = send(hwnd, LVM_GETITEMSTATE, index, LVIS_STATEIMAGEMASK.0 as isize) as u32;
    (state >> 12) == 2
}

pub fn list_add_column(hwnd: HWND, index: i32, title: &str, width: i32) {
    let mut t = wide(title);
    let col = LVCOLUMNW {
        mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
        cx: width,
        pszText: windows::core::PWSTR(t.as_mut_ptr()),
        iSubItem: index,
        ..Default::default()
    };
    send(hwnd, LVM_INSERTCOLUMNW, index as usize, &col as *const _ as isize);
}

pub fn list_set_count(hwnd: HWND, count: usize) {
    send(hwnd, LVM_SETITEMCOUNT, count, (LVSICF_NOINVALIDATEALL | LVSICF_NOSCROLL) as isize);
    invalidate(hwnd);
}

pub fn list_selected(hwnd: HWND) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = -1isize;
    loop {
        i = send(hwnd, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize);
        if i < 0 {
            break;
        }
        out.push(i as usize);
    }
    out
}

pub fn list_set_selected(hwnd: HWND, index: Option<usize>, selected: bool) {
    let item = LVITEMW {
        stateMask: LVIS_SELECTED,
        state: if selected { LVIS_SELECTED } else { LIST_VIEW_ITEM_STATE_FLAGS(0) },
        ..Default::default()
    };
    // Index -1 applies the state to every row.
    send(hwnd, LVM_SETITEMSTATE, index.map(|i| i as usize).unwrap_or(usize::MAX), &item as *const _ as isize);
}

pub fn list_focus_row(hwnd: HWND, index: usize) {
    let item = LVITEMW {
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
        state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
        ..Default::default()
    };
    send(hwnd, LVM_SETITEMSTATE, index, &item as *const _ as isize);
    send(hwnd, LVM_ENSUREVISIBLE, index, 0);
}

/// Sets the row height through a 1-pixel-wide image list, the documented
/// way to size report rows.
pub fn list_set_row_height(hwnd: HWND, height: i32) {
    unsafe {
        let il = ImageList_Create(1, height, ILC_COLOR32, 1, 0);
        if !il.is_invalid() {
            let old = send(hwnd, LVM_SETIMAGELIST, LVSIL_SMALL as usize, il.0 as isize);
            if old != 0 {
                let _ = ImageList_Destroy(Some(HIMAGELIST(old)));
            }
        }
    }
}

/// Copies `text` into the buffer supplied by LVN_GETDISPINFO.
pub fn fill_dispinfo(item: &mut LVITEMW, text: &str) {
    if item.pszText.is_null() || item.cchTextMax <= 0 {
        return;
    }
    let max = item.cchTextMax as usize - 1;
    let mut n = 0;
    for unit in text.encode_utf16().take(max) {
        unsafe { *item.pszText.0.add(n) = unit };
        n += 1;
    }
    unsafe { *item.pszText.0.add(n) = 0 };
}

pub fn message_box(owner: HWND, title: &str, text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(Some(owner), PCWSTR(wide(text).as_ptr()), PCWSTR(wide(title).as_ptr()), style) }
}

pub fn confirm(owner: HWND, title: &str, text: &str) -> bool {
    message_box(owner, title, text, MB_OKCANCEL | MB_ICONQUESTION) == IDOK
}

pub fn info(owner: HWND, title: &str, text: &str) {
    message_box(owner, title, text, MB_OK | MB_ICONINFORMATION);
}

/// "3 min ago" style text for a Unix time, or a dash when unknown.
pub fn ago(now: i64, at: Option<i64>) -> String {
    let Some(at) = at else { return "\u{2014}".to_string() };
    let d = (now - at).max(0);
    match d {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} d ago", d / 86_400),
    }
}

pub fn mb(bytes: u64) -> String {
    format!("{} MB", bytes / (1024 * 1024))
}

pub fn loword(v: usize) -> u16 {
    (v & 0xffff) as u16
}

pub fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xffff) as u16
}
