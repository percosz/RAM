//! Placeholder avatars: up to two initials on a deterministic colour, drawn
//! by the account list. No network or image decoding.

/// Up to two upper-case initials for a display name.
pub fn initials(name: &str) -> String {
    let mut out = String::new();
    for word in name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).take(2) {
        if let Some(c) = word.chars().next() {
            out.push(c.to_ascii_uppercase());
        }
    }
    if out.is_empty() {
        if let Some(c) = name.chars().find(|c| c.is_alphanumeric()) {
            out.push(c.to_ascii_uppercase());
        }
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

/// A deterministic, pleasant avatar colour as a COLORREF (0x00BBGGRR) for a
/// seed such as an account id.
pub fn color(seed: u64) -> u32 {
    // Distinct, muted hues that read on both light and dark backgrounds.
    const COLORS: &[u32] = &[
        0x00B5733E, // blue
        0x004FA86C, // green
        0x003E6FB5, // indigo
        0x00A8674F, // slate
        0x00B5733E, // teal
        0x004F7BA8, // amber
        0x00924FA8, // purple
        0x00A84F6E, // rose
        0x004FA89C, // cyan
        0x006EA84F, // lime
    ];
    COLORS[(seed as usize) % COLORS.len()]
}
