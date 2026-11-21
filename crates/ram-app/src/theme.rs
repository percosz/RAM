//! Themes: System, Light, Dark and a muted Rogue Lineage-inspired palette,
//! with an optional custom accent. High-contrast mode always wins and uses
//! the system colors unchanged.

use crate::ui::{self, wide};
use ram_core::model::{Settings, ThemeKind};
use std::ffi::c_void;
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Colors as 0x00BBGGRR COLORREF values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub high_contrast: bool,
    pub window: u32,
    pub surface: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub button: u32,
    pub button_hot: u32,
    pub good: u32,
    pub warn: u32,
    pub bad: u32,
}

const fn rgb(hex: u32) -> u32 {
    ((hex & 0xff) << 16) | (hex & 0xff00) | ((hex >> 16) & 0xff)
}

const LIGHT: Palette = Palette {
    dark: false,
    high_contrast: false,
    window: rgb(0xF5F5F4),
    surface: rgb(0xFFFFFF),
    text: rgb(0x1C1C1C),
    muted: rgb(0x5F6368),
    border: rgb(0xD4D4D2),
    accent: rgb(0x2F6FB5),
    accent_text: rgb(0xFFFFFF),
    button: rgb(0xFFFFFF),
    button_hot: rgb(0xEBEBEA),
    good: rgb(0x1E7B3C),
    warn: rgb(0x8A5A00),
    bad: rgb(0xB3261E),
};

const DARK: Palette = Palette {
    dark: true,
    high_contrast: false,
    window: rgb(0x1E1F22),
    surface: rgb(0x26282C),
    text: rgb(0xE6E6E6),
    muted: rgb(0x9DA2A8),
    border: rgb(0x3C3F45),
    accent: rgb(0x5B9BE0),
    accent_text: rgb(0x101214),
    button: rgb(0x303338),
    button_hot: rgb(0x3A3E44),
    good: rgb(0x6CC58A),
    warn: rgb(0xE0B05A),
    bad: rgb(0xF08A80),
};

/// Muted parchment-and-moss tones on a cold dark stone base.
const LINEAGE: Palette = Palette {
    dark: true,
    high_contrast: false,
    window: rgb(0x1B1D1A),
    surface: rgb(0x232621),
    text: rgb(0xDCD6C4),
    muted: rgb(0x9A9684),
    border: rgb(0x3A3D34),
    accent: rgb(0x8FA37A),
    accent_text: rgb(0x14160F),
    button: rgb(0x2C3029),
    button_hot: rgb(0x363B32),
    good: rgb(0x8FBF7A),
    warn: rgb(0xD0A45C),
    bad: rgb(0xD98A7A),
};

// Named palettes. Each is built from the palette author's own colours.
const fn pal(dark: bool, window: u32, surface: u32, text: u32, muted: u32, border: u32, accent: u32, accent_text: u32, button: u32, button_hot: u32, good: u32, warn: u32, bad: u32) -> Palette {
    Palette {
        dark,
        high_contrast: false,
        window: rgb(window),
        surface: rgb(surface),
        text: rgb(text),
        muted: rgb(muted),
        border: rgb(border),
        accent: rgb(accent),
        accent_text: rgb(accent_text),
        button: rgb(button),
        button_hot: rgb(button_hot),
        good: rgb(good),
        warn: rgb(warn),
        bad: rgb(bad),
    }
}

// Catppuccin — https://github.com/catppuccin/catppuccin (MIT).
const MOCHA: Palette = pal(true, 0x1E1E2E, 0x24243A, 0xCDD6F4, 0xA6ADC8, 0x45475A, 0xCBA6F7, 0x1E1E2E, 0x313244, 0x45475A, 0xA6E3A1, 0xF9E2AF, 0xF38BA8);
const MACCHIATO: Palette = pal(true, 0x24273A, 0x2A2D43, 0xCAD3F5, 0xA5ADCB, 0x494D64, 0xC6A0F6, 0x24273A, 0x363A4F, 0x494D64, 0xA6DA95, 0xEED49F, 0xED8796);
const FRAPPE: Palette = pal(true, 0x303446, 0x363A4C, 0xC6D0F5, 0xA5ADCE, 0x51576D, 0xCA9EE6, 0x303446, 0x414559, 0x51576D, 0xA6D189, 0xE5C890, 0xE78284);
const LATTE: Palette = pal(false, 0xEFF1F5, 0xFFFFFF, 0x4C4F69, 0x6C6F85, 0xCCD0DA, 0x8839EF, 0xFFFFFF, 0xE6E9EF, 0xDCE0E8, 0x40A02B, 0xDF8E1D, 0xD20F39);
// Nord — https://www.nordtheme.com (MIT).
const NORD: Palette = pal(true, 0x2E3440, 0x343B4A, 0xECEFF4, 0xA6B0C0, 0x434C5E, 0x88C0D0, 0x2E3440, 0x3B4252, 0x434C5E, 0xA3BE8C, 0xEBCB8B, 0xBF616A);
// Dracula — https://draculatheme.com (MIT).
const DRACULA: Palette = pal(true, 0x282A36, 0x2E303E, 0xF8F8F2, 0x9AA2C4, 0x44475A, 0xBD93F9, 0x282A36, 0x343746, 0x44475A, 0x50FA7B, 0xF1FA8C, 0xFF5555);
// Gruvbox — https://github.com/morhetz/gruvbox (MIT).
const GRUVBOX: Palette = pal(true, 0x282828, 0x32302F, 0xEBDBB2, 0xA89984, 0x504945, 0xFE8019, 0x282828, 0x3C3836, 0x504945, 0xB8BB26, 0xFABD2F, 0xFB4934);
// Rosé Pine — https://rosepinetheme.com (MIT). Pink accent, alyae-like.
const ROSE_PINE: Palette = pal(true, 0x191724, 0x1F1D2E, 0xE0DEF4, 0x908CAA, 0x403D52, 0xEB6F92, 0x191724, 0x26233A, 0x403D52, 0x9CCFD8, 0xF6C177, 0xEB6F92);

fn sys(index: SYS_COLOR_INDEX) -> u32 {
    unsafe { GetSysColor(index) }
}

fn high_contrast_on() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    unsafe {
        SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, Some(&mut hc as *mut _ as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
            .is_ok()
            && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
}

/// True when Windows is set to dark app mode.
pub fn system_prefers_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut c_void),
            Some(&mut size),
        )
    };
    ok.is_ok() && value == 0
}

pub fn palette(settings: &Settings) -> Palette {
    if high_contrast_on() {
        return Palette {
            dark: false,
            high_contrast: true,
            window: sys(COLOR_WINDOW),
            surface: sys(COLOR_WINDOW),
            text: sys(COLOR_WINDOWTEXT),
            muted: sys(COLOR_WINDOWTEXT),
            border: sys(COLOR_WINDOWTEXT),
            accent: sys(COLOR_HIGHLIGHT),
            accent_text: sys(COLOR_HIGHLIGHTTEXT),
            button: sys(COLOR_BTNFACE),
            button_hot: sys(COLOR_BTNFACE),
            good: sys(COLOR_WINDOWTEXT),
            warn: sys(COLOR_WINDOWTEXT),
            bad: sys(COLOR_WINDOWTEXT),
        };
    }
    let mut p = match settings.theme {
        ThemeKind::Light => LIGHT,
        ThemeKind::Dark => DARK,
        ThemeKind::Mocha => MOCHA,
        ThemeKind::Macchiato => MACCHIATO,
        ThemeKind::Frappe => FRAPPE,
        ThemeKind::Latte => LATTE,
        ThemeKind::Nord => NORD,
        ThemeKind::Dracula => DRACULA,
        ThemeKind::Gruvbox => GRUVBOX,
        ThemeKind::RosePine => ROSE_PINE,
        ThemeKind::Lineage => LINEAGE,
        ThemeKind::System => {
            if system_prefers_dark() {
                DARK
            } else {
                LIGHT
            }
        }
    };
    if let Some(accent) = settings.accent {
        p.accent = rgb(accent & 0xFFFFFF);
        p.accent_text = if luminance(p.accent) > 140 { rgb(0x101214) } else { rgb(0xFFFFFF) };
    }
    p
}

fn luminance(c: u32) -> u32 {
    let (r, g, b) = (c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff);
    (r * 299 + g * 587 + b * 114) / 1000
}

/// Brushes for the current palette. Recreated on theme change.
pub struct Brushes {
    pub window: HBRUSH,
    pub surface: HBRUSH,
    pub border: HBRUSH,
}

impl Brushes {
    pub fn new(p: &Palette) -> Self {
        unsafe {
            Brushes {
                window: CreateSolidBrush(COLORREF(p.window)),
                surface: CreateSolidBrush(COLORREF(p.surface)),
                border: CreateSolidBrush(COLORREF(p.border)),
            }
        }
    }

    pub fn release(&mut self) {
        unsafe {
            let _ = DeleteObject(self.window.into());
            let _ = DeleteObject(self.surface.into());
            let _ = DeleteObject(self.border.into());
        }
    }
}

/// Dark title bar for dark palettes.
pub fn apply_to_frame(hwnd: HWND, p: &Palette) {
    let on: i32 = (p.dark && !p.high_contrast) as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const _ as *const c_void, 4);
    }
}

fn set_theme(hwnd: HWND, name: PCWSTR) {
    unsafe {
        let _ = SetWindowTheme(hwnd, name, PCWSTR::null());
    }
}

/// Styles one child control for the palette, based on its window class.
pub fn apply_to_control(hwnd: HWND, p: &Palette) {
    let mut buf = [0u16; 32];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]).to_ascii_lowercase();
    let dark = p.dark && !p.high_contrast;
    match class.as_str() {
        "syslistview32" => {
            set_theme(hwnd, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") });
            ui::send(hwnd, LVM_SETBKCOLOR, 0, p.surface as isize);
            ui::send(hwnd, LVM_SETTEXTBKCOLOR, 0, p.surface as isize);
            ui::send(hwnd, LVM_SETTEXTCOLOR, 0, p.text as isize);
            let header = HWND(ui::send(hwnd, LVM_GETHEADER, 0, 0) as *mut c_void);
            if !header.is_invalid() {
                set_theme(header, if dark { w!("DarkMode_ItemsView") } else { w!("ItemsView") });
            }
        }
        "edit" => {
            // Multiline edits get the Explorer style so their scroll bars
            // follow the theme.
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
            let multiline = style & ES_MULTILINE as u32 != 0;
            set_theme(
                hwnd,
                match (dark, multiline) {
                    (true, true) => w!("DarkMode_Explorer"),
                    (true, false) => w!("DarkMode_CFD"),
                    (false, true) => w!("Explorer"),
                    (false, false) => w!("CFD"),
                },
            );
        }
        "combobox" => set_theme(hwnd, if dark { w!("DarkMode_CFD") } else { w!("CFD") }),
        "button" => {
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
            // Themed check boxes ignore WM_CTLCOLOR text colors; the classic
            // renderer honors them.
            if style & 0xF == BS_AUTOCHECKBOX as u32 && !p.high_contrast {
                set_theme(hwnd, PCWSTR(wide("").as_ptr()));
                unsafe {
                    let _ = SetWindowTheme(hwnd, w!(""), w!(""));
                }
            }
        }
        _ => {}
    }
    ui::invalidate(hwnd);
}

/// Styles every descendant of `root`.
pub fn apply_to_tree(root: HWND, p: &Palette) {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let p = &*(lparam.0 as *const Palette);
        apply_to_control(hwnd, p);
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(Some(root), Some(each), LPARAM(p as *const _ as isize));
    }
    ui::invalidate(root);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    /// The primary action of a view.
    Accent,
    /// A navigation tab; `true` when it is the current view.
    Nav(bool),
}

/// Blends two COLORREFs: `t` percent of `a` over `b`.
fn mix(a: u32, b: u32, t: u32) -> u32 {
    let ch = |sh: u32| {
        let (ca, cb) = ((a >> sh) & 0xff, (b >> sh) & 0xff);
        ((ca * t + cb * (100 - t)) / 100) & 0xff
    };
    ch(0) | (ch(8) << 8) | (ch(16) << 16)
}

/// Fills a rounded rectangle with `fill`, outlined in `border` (equal to
/// `fill` for no visible edge). `radius` of 0 draws square corners.
unsafe fn round_rect(hdc: HDC, rc: RECT, radius: i32, fill: u32, border: u32) {
    let brush = CreateSolidBrush(COLORREF(fill));
    let pen = CreatePen(PS_SOLID, 1, COLORREF(border));
    let ob = SelectObject(hdc, brush.into());
    let op = SelectObject(hdc, pen.into());
    if radius > 0 {
        let d = radius * 2;
        let _ = RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, d, d);
    } else {
        let _ = Rectangle(hdc, rc.left, rc.top, rc.right, rc.bottom);
    }
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());
}

/// Paints an owner-drawn button: rounded fills, a sidebar style for nav
/// items, and a visible keyboard focus ring.
pub fn draw_button(dis: &DRAWITEMSTRUCT, p: &Palette, kind: ButtonKind, font: HFONT, corner: i32) {
    unsafe {
        let hdc = dis.hDC;
        let rc = dis.rcItem;
        let pressed = (dis.itemState.0 & ODS_SELECTED.0) != 0;
        let disabled = (dis.itemState.0 & ODS_DISABLED.0) != 0;
        let focused = (dis.itemState.0 & ODS_FOCUS.0) != 0 && (dis.itemState.0 & ODS_NOFOCUSRECT.0) == 0;
        let radius = if p.high_contrast { 0 } else { corner };

        // Clear to the surrounding colour (nav sits on the sidebar surface).
        let surround = if matches!(kind, ButtonKind::Nav(_)) { p.surface } else { p.window };
        let clear = CreateSolidBrush(COLORREF(surround));
        FillRect(hdc, &rc, clear);
        let _ = DeleteObject(clear.into());

        let (fg, left_align) = match kind {
            ButtonKind::Nav(active) => {
                if active {
                    let wash = if p.dark { mix(p.accent, p.surface, 24) } else { mix(p.accent, p.surface, 16) };
                    round_rect(hdc, rc, radius, wash, wash);
                    let bar = RECT { left: rc.left, right: rc.left + 3, top: rc.top + 3, bottom: rc.bottom - 3 };
                    let accent = CreateSolidBrush(COLORREF(p.accent));
                    FillRect(hdc, &bar, accent);
                    let _ = DeleteObject(accent.into());
                    (p.text, true)
                } else if pressed {
                    round_rect(hdc, rc, radius, mix(p.text, p.surface, 8), mix(p.text, p.surface, 8));
                    (p.text, true)
                } else {
                    (p.muted, true)
                }
            }
            ButtonKind::Accent if !disabled => {
                let bg = if pressed { mix(p.accent, 0, 85) } else { p.accent };
                round_rect(hdc, rc, radius, bg, bg);
                (p.accent_text, false)
            }
            _ => {
                let bg = if pressed { p.button_hot } else { p.button };
                round_rect(hdc, rc, radius, bg, p.border);
                (if disabled { p.muted } else { p.text }, false)
            }
        };

        let text = ui::get_text(dis.hwndItem);
        let mut wtext: Vec<u16> = text.encode_utf16().collect();
        let old_font = SelectObject(hdc, font.into());
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(fg));
        let mut trc = rc;
        let flags = if left_align {
            trc.left += (rc.bottom - rc.top) / 2;
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
        } else {
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
        };
        DrawTextW(hdc, &mut wtext, &mut trc, flags);
        if focused {
            let mut frc = rc;
            let _ = InflateRect(&mut frc, -3, -3);
            SetTextColor(hdc, COLORREF(p.text));
            SetBkColor(hdc, COLORREF(surround));
            let _ = DrawFocusRect(hdc, &frc);
        }
        SelectObject(hdc, old_font);
    }
}

/// Makes a list's column header use the palette's text color. The header
/// sends its custom-draw notifications to the list, so the list is
/// subclassed to answer them.
pub fn subclass_list(list: HWND) {
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_NOTIFY {
            let hdr = &*(lparam.0 as *const NMHDR);
            if hdr.code == NM_CUSTOMDRAW && hdr.hwndFrom.0 as isize == ui::send(hwnd, LVM_GETHEADER, 0, 0) {
                if let Some(app) = crate::app::try_app() {
                    let cd = &*(lparam.0 as *const NMCUSTOMDRAW);
                    match cd.dwDrawStage.0 {
                        0x1 => return LRESULT(CDRF_NOTIFYITEMDRAW as isize),
                        0x10001 => {
                            SetTextColor(cd.hdc, COLORREF(app.palette.text));
                            return LRESULT(CDRF_DODEFAULT as isize);
                        }
                        _ => {}
                    }
                }
            }
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
    unsafe {
        let _ = SetWindowSubclass(list, Some(proc), 1, 0);
    }
}

/// Parses "#RRGGBB" or "RRGGBB".
pub fn parse_hex(text: &str) -> Option<u32> {
    let t = text.trim().trim_start_matches('#');
    (t.len() == 6).then(|| u32::from_str_radix(t, 16).ok()).flatten()
}
