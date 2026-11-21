//! Graphics / FPS settings applied around an alt-account client launch.
//!
//! Two mechanisms, both written right before a launch and undone once the
//! client has read them:
//!
//! 1. `GlobalBasicSettings_13.xml` (Roblox's own user settings): the FPS cap
//!    (`FramerateCap`) and the quality level (`GraphicsQualityLevel` +
//!    `SavedQualityLevel`). Frame-rate FastFlags are refused by Roblox's
//!    allowlist, so the cap has to go through this file.
//! 2. `ClientAppSettings.json` (FastFlags, Bloxstrap style): lighter rendering
//!    (LOD distances, MSAA, texture quality, render API, ...) plus the user's
//!    own flags, which are appended last so they win.
//!
//! Main accounts are never touched. Everything here is self-contained so the
//! module can be compiled and tested on its own.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Settings model
// ---------------------------------------------------------------------------

/// Forced MSAA sample count (`FIntDebugForceMSAASamples`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Msaa {
    /// Leave the client's choice alone.
    Default,
    #[default]
    X1,
    X2,
    X4,
}

/// Texture quality override (`DFIntTextureQualityOverride`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Texture {
    /// Leave the client's choice alone.
    Default,
    #[default]
    Lowest,
    Low,
    Medium,
    High,
}

/// Preferred rendering API (`FFlagDebugGraphicsPrefer*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RenderMode {
    #[default]
    Default,
    D3D11,
    Vulkan,
    OpenGl,
}

/// The individual "lighter rendering" FastFlag switches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderFlags {
    pub msaa: Msaa,
    pub texture: Texture,
    pub mode: RenderMode,
    pub gray_sky: bool,
    pub no_grass: bool,
    pub lowest_frm: bool,
    pub pause_voxelizer: bool,
}

impl Default for RenderFlags {
    fn default() -> Self {
        Self {
            msaa: Msaa::default(),
            texture: Texture::default(),
            mode: RenderMode::default(),
            gray_sky: false,
            no_grass: true,
            lowest_frm: true,
            pause_voxelizer: false,
        }
    }
}

/// Global graphics settings for alt accounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    /// Master switch. When off nothing is ever written.
    pub enabled: bool,
    /// 0 = leave the user's own cap; otherwise 5..=360.
    pub fps_cap: u32,
    /// Lowest graphics quality via the XML (`GraphicsQualityLevel` 1 +
    /// `SavedQualityLevel` 1).
    pub low_graphics: bool,
    /// Lighter rendering via FastFlags (`render`).
    pub lighter_rendering: bool,
    pub render: RenderFlags,
    /// The user's own FastFlags, appended last so they win.
    pub custom_flags: BTreeMap<String, String>,
    /// Set Roblox's own volume to 0 for alt clients (`MasterVolume` in the
    /// XML), restored like the other values.
    pub mute_volume: bool,
}

impl Default for GraphicsSettings {
    /// Roblox's defaults: nothing is changed until the user picks something.
    fn default() -> Self {
        Self {
            enabled: false,
            fps_cap: 0,
            low_graphics: false,
            lighter_rendering: false,
            render: RenderFlags::default(),
            custom_flags: BTreeMap::new(),
            mute_volume: false,
        }
    }
}

/// Per-account overrides.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountGraphics {
    /// Overrides [`GraphicsSettings::fps_cap`] when set (0 = leave the cap).
    pub fps_cap: Option<u32>,
    /// Disables low graphics and lighter rendering for this account; custom
    /// flags still apply.
    pub full_quality: bool,
}

/// What actually gets applied for one launch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effective {
    /// 0 = leave the user's cap.
    pub fps_cap: u32,
    pub low_graphics: bool,
    /// `Some` when lighter rendering is on for this launch.
    pub lighter: Option<RenderFlags>,
    pub custom: BTreeMap<String, String>,
    /// Roblox volume 0 for this launch.
    #[serde(default)]
    pub mute: bool,
}

/// Clamps an FPS cap to 0 (leave alone) or 5..=360.
pub fn clamp_fps(cap: u32) -> u32 {
    if cap == 0 {
        0
    } else {
        cap.clamp(5, 360)
    }
}

/// Resolves the settings for one launch. `None` means: touch nothing (the
/// feature is off, or this is a Main account).
pub fn effective(g: &GraphicsSettings, acct: &AccountGraphics, is_main: bool) -> Option<Effective> {
    if !g.enabled || is_main {
        return None;
    }
    let fps_cap = clamp_fps(acct.fps_cap.unwrap_or(g.fps_cap));
    let low_graphics = g.low_graphics && !acct.full_quality;
    let lighter = if g.lighter_rendering && !acct.full_quality {
        Some(g.render.clone())
    } else {
        None
    };
    Some(Effective {
        fps_cap,
        low_graphics,
        lighter,
        custom: g.custom_flags.clone(),
        mute: g.mute_volume && !acct.full_quality,
    })
}

// ---------------------------------------------------------------------------
// FastFlags
// ---------------------------------------------------------------------------

/// The three mutually exclusive render-API preference flags.
pub const RENDER_MODE_FLAGS: [&str; 3] = [
    "FFlagDebugGraphicsPreferD3D11",
    "FFlagDebugGraphicsPreferVulkan",
    "FFlagDebugGraphicsPreferOpenGL",
];

/// Builds the FastFlag set for one launch (lighter-rendering flags first,
/// then the user's custom flags so they win).
pub fn flag_set(e: &Effective) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        out.insert(k.to_string(), v.to_string());
    };
    if let Some(r) = &e.lighter {
        put("DFIntCSGLevelOfDetailSwitchingDistance", "0");
        put("DFIntCSGLevelOfDetailSwitchingDistanceL12", "0");
        put("DFIntCSGLevelOfDetailSwitchingDistanceL23", "0");
        put("DFIntCSGLevelOfDetailSwitchingDistanceL34", "0");
        match r.msaa {
            Msaa::Default => {}
            Msaa::X1 => put("FIntDebugForceMSAASamples", "1"),
            Msaa::X2 => put("FIntDebugForceMSAASamples", "2"),
            Msaa::X4 => put("FIntDebugForceMSAASamples", "4"),
        }
        let texture = match r.texture {
            Texture::Default => None,
            Texture::Lowest => Some("0"),
            Texture::Low => Some("1"),
            Texture::Medium => Some("2"),
            Texture::High => Some("3"),
        };
        if let Some(level) = texture {
            put("DFFlagTextureQualityOverrideEnabled", "True");
            put("DFIntTextureQualityOverride", level);
        }
        match r.mode {
            RenderMode::Default => {}
            RenderMode::D3D11 => put("FFlagDebugGraphicsPreferD3D11", "True"),
            RenderMode::Vulkan => put("FFlagDebugGraphicsPreferVulkan", "True"),
            RenderMode::OpenGl => put("FFlagDebugGraphicsPreferOpenGL", "True"),
        }
        if r.gray_sky {
            put("FFlagDebugSkyGray", "True");
        }
        if r.no_grass {
            put("FIntFRMMinGrassDistance", "0");
            put("FIntFRMMaxGrassDistance", "0");
        }
        if r.lowest_frm {
            put("DFIntDebugFRMQualityLevelOverride", "1");
        }
        if r.pause_voxelizer {
            put("DFFlagDebugPauseVoxelizer", "True");
        }
    }
    for (k, v) in &e.custom {
        put(k, v);
    }
    out
}

/// Checks user-supplied FastFlags: at most 500 entries, names of 1..=128
/// characters from `[A-Za-z0-9_]`, values of at most 1024 characters.
pub fn validate_custom(flags: &BTreeMap<String, String>) -> Result<(), String> {
    if flags.len() > 500 {
        return Err(format!("too many custom flags ({}; the limit is 500)", flags.len()));
    }
    for (name, value) in flags {
        let len = name.chars().count();
        if len == 0 || len > 128 {
            return Err(format!("flag name {name:?} must be 1 to 128 characters long"));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("flag name {name:?} may only contain letters, digits and underscores"));
        }
        if value.chars().count() > 1024 {
            return Err(format!("the value of {name} is longer than 1024 characters"));
        }
    }
    Ok(())
}

/// Merges `set` over an existing `ClientAppSettings.json` text. A blank text
/// counts as `{}`; anything that is not a JSON object is an error. Choosing a
/// render API strips the other two preference flags. Output is pretty JSON
/// (2-space indent, sorted keys, trailing newline).
pub fn merge_flags(existing: &str, set: &BTreeMap<String, String>) -> Result<String, String> {
    let mut map: BTreeMap<String, serde_json::Value> = if existing.trim().is_empty() {
        BTreeMap::new()
    } else {
        match serde_json::from_str::<serde_json::Value>(existing) {
            Ok(serde_json::Value::Object(m)) => m.into_iter().collect(),
            Ok(_) => return Err("ClientAppSettings.json is not a JSON object".to_string()),
            Err(e) => return Err(format!("ClientAppSettings.json is not valid JSON: {e}")),
        }
    };
    if set.keys().any(|k| RENDER_MODE_FLAGS.contains(&k.as_str())) {
        for flag in RENDER_MODE_FLAGS {
            if !set.contains_key(flag) {
                map.remove(flag);
            }
        }
    }
    for (k, v) in set {
        map.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    let mut out = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    out.push('\n');
    Ok(out)
}

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn io_err(what: &str, path: &Path, e: io::Error) -> String {
    format!("cannot {what} {}: {e}", path.display())
}

/// Reads a file; a missing file is `None`.
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err("read", path, e)),
    }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("delete", path, e)),
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| io_err("create directory", parent, e))?;
        }
    }
    Ok(())
}

/// Writes through `<path>.ram-tmp` and renames over the target.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = with_suffix(path, ".ram-tmp");
    fs::write(&tmp, bytes).map_err(|e| io_err("write", &tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err("replace", path, e)
    })
}

// ---------------------------------------------------------------------------
// ClientAppSettings.json handling
// ---------------------------------------------------------------------------

/// A `ClientAppSettings.json` on disk. The caller decides where it lives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagFile {
    pub path: PathBuf,
}

/// Handle to undo a [`FlagFile::apply`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagUndo {
    pub path: PathBuf,
}

/// Suffix of the backup written next to the flag file. Its first byte is `1`
/// (followed by the original bytes) or `0` (there was no file).
pub const FLAG_BACKUP_SUFFIX: &str = ".ram-before";

impl FlagFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Path of the backup written by [`FlagFile::apply`].
    pub fn backup_path(&self) -> PathBuf {
        with_suffix(&self.path, FLAG_BACKUP_SUFFIX)
    }

    /// Merges `set` into the file. A leftover backup from a crash is restored
    /// first; the current content is backed up; the merged content is written
    /// atomically. Nothing is written when the existing file is not a JSON
    /// object.
    pub fn apply(&self, set: &BTreeMap<String, String>) -> Result<FlagUndo, String> {
        self.put_back()?;
        let original = read_optional(&self.path)?;
        let existing = match &original {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => String::new(),
        };
        let merged = merge_flags(&existing, set)?;
        let mut backup = Vec::with_capacity(original.as_ref().map_or(1, |b| b.len() + 1));
        match &original {
            Some(b) => {
                backup.push(1);
                backup.extend_from_slice(b);
            }
            None => backup.push(0),
        }
        ensure_parent(&self.path)?;
        write_atomic(&self.backup_path(), &backup)?;
        write_atomic(&self.path, merged.as_bytes())?;
        Ok(FlagUndo {
            path: self.path.clone(),
        })
    }

    /// Restores the backup if there is one (original bytes written back, or
    /// the flag file deleted when there was none) and deletes the backup.
    /// Does nothing when no backup exists.
    pub fn put_back(&self) -> Result<(), String> {
        let backup_path = self.backup_path();
        let Some(backup) = read_optional(&backup_path)? else {
            return Ok(());
        };
        match backup.first() {
            Some(1) => write_atomic(&self.path, &backup[1..])?,
            Some(0) => remove_if_exists(&self.path)?,
            _ => {
                remove_if_exists(&backup_path)?;
                return Err(format!(
                    "backup {} is corrupt; the flag file was left as it is",
                    backup_path.display()
                ));
            }
        }
        remove_if_exists(&backup_path)
    }
}

impl FlagUndo {
    /// Same as [`FlagFile::put_back`] on the recorded path.
    pub fn put_back(self) -> Result<(), String> {
        FlagFile { path: self.path }.put_back()
    }
}

/// Puts back every flag file that still has a backup (app start / exit).
/// Returns one message per failure.
pub fn restore_leftovers(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| FlagFile { path: p.clone() }.put_back().err())
        .collect()
}

// ---------------------------------------------------------------------------
// GlobalBasicSettings_13.xml handling
// ---------------------------------------------------------------------------

/// Minimal valid `GlobalBasicSettings_13.xml`.
pub const EMPTY_XML: &str = "<roblox xmlns:xmime=\"http://www.w3.org/2005/05/xmlmime\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"http://www.roblox.com/roblox.xsd\" version=\"4\">\n\t<Item class=\"UserGameSettings\" referent=\"RBX0\">\n\t\t<Properties>\n\t\t</Properties>\n\t</Item>\n</roblox>\n";

/// The three values we care about. `None` = absent (when read) or "leave it"
/// (when written).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct XmlValues {
    pub framerate_cap: Option<i64>,
    pub quality_level: Option<i64>,
    pub saved_quality: Option<i64>,
    /// `MasterVolume` in thousandths (0 = silent, 1000 = full).
    pub master_volume: Option<i64>,
}

const XML_FRAMERATE: (&str, &str) = ("int", "FramerateCap");
const XML_QUALITY: (&str, &str) = ("int", "GraphicsQualityLevel");
const XML_SAVED: (&str, &str) = ("token", "SavedQualityLevel");
const XML_VOLUME: (&str, &str) = ("float", "MasterVolume");

/// Reads a float element as thousandths.
fn xml_get_milli(text: &str, (elem, name): (&str, &str)) -> Option<i64> {
    let r = xml_value_range(text, elem, name)?;
    let v: f64 = text[r].trim().parse().ok()?;
    v.is_finite().then(|| (v * 1000.0).round() as i64)
}

/// Thousandths as the shortest decimal Roblox accepts ("0", "1", "0.5").
fn milli_text(milli: i64) -> String {
    if milli % 1000 == 0 {
        (milli / 1000).to_string()
    } else {
        let s = format!("{:.3}", milli as f64 / 1000.0);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn xml_set_text(text: &mut String, (elem, name): (&str, &str), value: &str) {
    if let Some(r) = xml_value_range(text, elem, name) {
        text.replace_range(r, value);
        return;
    }
    let Some(close) = text.find("</Properties>") else {
        return;
    };
    let element = format!("{}{value}</{elem}>", xml_open_tag(elem, name));
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    if text[line_start..close].trim().is_empty() {
        text.insert_str(line_start, &format!("\t\t\t{element}\n"));
    } else {
        text.insert_str(close, &format!("\n\t\t\t{element}\n\t\t"));
    }
}

fn xml_open_tag(elem: &str, name: &str) -> String {
    format!("<{elem} name=\"{name}\">")
}

/// Byte range of the text between `<elem name="name">` and `</elem>`.
fn xml_value_range(text: &str, elem: &str, name: &str) -> Option<Range<usize>> {
    let open = xml_open_tag(elem, name);
    let close = format!("</{elem}>");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find(&close)?;
    Some(start..end)
}

fn xml_get_one(text: &str, (elem, name): (&str, &str)) -> Option<i64> {
    let r = xml_value_range(text, elem, name)?;
    text[r].trim().parse().ok()
}

/// Replaces the element's value, or inserts the element before the first
/// `</Properties>` (on its own tab-indented line when `</Properties>` starts
/// a line).
fn xml_set_one(text: &mut String, (elem, name): (&str, &str), value: i64) {
    if let Some(r) = xml_value_range(text, elem, name) {
        text.replace_range(r, &value.to_string());
        return;
    }
    let Some(close) = text.find("</Properties>") else {
        return;
    };
    let element = format!("{}{value}</{elem}>", xml_open_tag(elem, name));
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    if text[line_start..close].trim().is_empty() {
        text.insert_str(line_start, &format!("\t\t\t{element}\n"));
    } else {
        text.insert_str(close, &format!("\n\t\t\t{element}\n\t\t"));
    }
}

/// Removes the element (with its whole line when it is alone on it).
fn xml_remove_one(text: &mut String, (elem, name): (&str, &str)) {
    let Some(r) = xml_value_range(text, elem, name) else {
        return;
    };
    let elem_start = r.start - xml_open_tag(elem, name).len();
    let elem_end = r.end + format!("</{elem}>").len();
    let line_start = text[..elem_start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[elem_end..]
        .find('\n')
        .map_or(text.len(), |i| elem_end + i + 1);
    let alone = text[line_start..elem_start].trim().is_empty()
        && text[elem_end..line_end].trim().is_empty();
    if alone {
        text.replace_range(line_start..line_end, "");
    } else {
        text.replace_range(elem_start..elem_end, "");
    }
}

/// Reads the three values textually.
pub fn xml_get(text: &str) -> XmlValues {
    XmlValues {
        framerate_cap: xml_get_one(text, XML_FRAMERATE),
        quality_level: xml_get_one(text, XML_QUALITY),
        saved_quality: xml_get_one(text, XML_SAVED),
        master_volume: xml_get_milli(text, XML_VOLUME),
    }
}

/// True when the text looks like a complete settings file.
fn xml_is_complete(text: &str) -> bool {
    text.contains("</Properties>") && text.trim().ends_with("</roblox>")
}

/// Writes each `Some` value into the text (replace or insert); everything
/// else stays byte-identical. A text without `</Properties>` or not ending in
/// `</roblox>` (half-written) is replaced by [`EMPTY_XML`] first.
pub fn xml_set(text: &str, v: &XmlValues) -> String {
    let mut out = if xml_is_complete(text) {
        text.to_string()
    } else {
        EMPTY_XML.to_string()
    };
    if let Some(n) = v.framerate_cap {
        xml_set_one(&mut out, XML_FRAMERATE, n);
    }
    if let Some(n) = v.quality_level {
        xml_set_one(&mut out, XML_QUALITY, n);
    }
    if let Some(n) = v.saved_quality {
        xml_set_one(&mut out, XML_SAVED, n);
    }
    if let Some(n) = v.master_volume {
        xml_set_text(&mut out, XML_VOLUME, &milli_text(n));
    }
    out
}

/// The values to write for a launch, given what the file holds now.
pub fn xml_for_launch(e: &Effective, current: &XmlValues) -> XmlValues {
    XmlValues {
        framerate_cap: if e.fps_cap > 0 {
            Some(i64::from(e.fps_cap))
        } else {
            current.framerate_cap
        },
        quality_level: if e.low_graphics { Some(1) } else { current.quality_level },
        saved_quality: if e.low_graphics { Some(1) } else { current.saved_quality },
        master_volume: if e.mute { Some(0) } else { current.master_volume },
    }
}

/// A `GlobalBasicSettings_13.xml` on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlFile {
    pub path: PathBuf,
}

/// What [`XmlFile::apply`] changed: the user's values before, and ours.
/// Persist it so the restore can finish after a crash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct XmlUndo {
    pub path: PathBuf,
    pub user: XmlValues,
    pub ours: XmlValues,
}

impl Default for XmlUndo {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            user: XmlValues::default(),
            ours: XmlValues::default(),
        }
    }
}

impl XmlFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn read_checked(&self) -> Result<(String, XmlValues), String> {
        let text = match read_optional(&self.path)? {
            Some(b) => String::from_utf8_lossy(&b).into_owned(),
            None => EMPTY_XML.to_string(),
        };
        let values = xml_get(&text);
        Ok((text, values))
    }

    /// Current text and values; a missing (or unreadable) file reads as
    /// [`EMPTY_XML`].
    pub fn read(&self) -> (String, XmlValues) {
        self.read_checked()
            .unwrap_or_else(|_| (EMPTY_XML.to_string(), XmlValues::default()))
    }

    /// Writes `want` (only the `Some` values), atomically and only when the
    /// text actually changes. Records the user's current values for the undo.
    pub fn apply(&self, want: &XmlValues) -> Result<XmlUndo, String> {
        let (text, user) = self.read_checked()?;
        let new_text = xml_set(&text, want);
        if new_text != text {
            ensure_parent(&self.path)?;
            write_atomic(&self.path, new_text.as_bytes())?;
        }
        Ok(XmlUndo {
            path: self.path.clone(),
            user,
            ours: want.clone(),
        })
    }
}

impl XmlUndo {
    /// Re-reads the file and, for each key we set, restores the user's value
    /// when the file still holds ours. A value that differs from both is a
    /// user edit and is left alone.
    pub fn put_back(&self) -> Result<(), String> {
        settle(&self.path, self).map(|_| ())
    }
}

/// The restore logic of [`XmlUndo::put_back`], for a watcher that runs after
/// clients exit (Roblox re-saves the whole XML on exit, leaking our values).
/// Returns `true` when it wrote the file. A missing file is nothing to do;
/// a half-written file is an error (try again later).
pub fn settle(path: &Path, undo: &XmlUndo) -> Result<bool, String> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(false);
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if !xml_is_complete(&text) {
        return Err(format!("{} is incomplete (still being written?)", path.display()));
    }
    let current = xml_get(&text);
    let mut out = text.clone();
    let keys = [
        (XML_FRAMERATE, undo.ours.framerate_cap, current.framerate_cap, undo.user.framerate_cap),
        (XML_QUALITY, undo.ours.quality_level, current.quality_level, undo.user.quality_level),
        (XML_SAVED, undo.ours.saved_quality, current.saved_quality, undo.user.saved_quality),
    ];
    for (key, ours, now, user) in keys {
        let Some(ours) = ours else { continue };
        if now != Some(ours) {
            continue;
        }
        match user {
            Some(v) => xml_set_one(&mut out, key, v),
            None => xml_remove_one(&mut out, key),
        }
    }
    if let Some(ours) = undo.ours.master_volume {
        if current.master_volume == Some(ours) {
            match undo.user.master_volume {
                Some(v) => xml_set_text(&mut out, XML_VOLUME, &milli_text(v)),
                None => xml_remove_one(&mut out, XML_VOLUME),
            }
        }
    }
    if out == text {
        return Ok(false);
    }
    write_atomic(path, out.as_bytes())?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Client log detection
// ---------------------------------------------------------------------------

/// Lines that show the client has read its settings (safe to put back).
pub const READ_MARKS: [&str; 3] = [
    "SceneManager: resizing main targets",
    "[FLog::Network] Replicator created",
    "LoadClientSettingsFromLocal",
];

/// True when the client log shows the settings have been read.
pub fn has_read_mark(log_text: &str) -> bool {
    READ_MARKS.iter().any(|m| log_text.contains(m))
}

/// True when `text` contains `needle` not immediately followed by a digit.
fn contains_bounded(text: &str, needle: &str) -> bool {
    text.match_indices(needle)
        .any(|(i, _)| !text[i + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// True when the head of a client log names the given browser tracker id.
pub fn names_tracker(log_head: &str, browser_tracker_id: &str) -> bool {
    let id = browser_tracker_id.trim();
    if id.is_empty() {
        return false;
    }
    [
        format!("BrowserTrackerId={id}"),
        format!("websiteBTId is {id}"),
        format!("BTID is overriden to {id}"),
    ]
    .iter()
    .any(|needle| contains_bounded(log_head, needle))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let p = std::env::temp_dir().join(format!(
                "ram-client-settings-{}-{tag}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn enabled() -> GraphicsSettings {
        GraphicsSettings {
            enabled: true,
            low_graphics: true,
            lighter_rendering: true,
            ..GraphicsSettings::default()
        }
    }

    fn lighter(render: RenderFlags) -> Effective {
        Effective {
            fps_cap: 0,
            low_graphics: false,
            lighter: Some(render),
            custom: BTreeMap::new(),
            mute: false,
        }
    }

    const SAMPLE_XML: &str = "<roblox xmlns:xmime=\"http://www.w3.org/2005/05/xmlmime\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"http://www.roblox.com/roblox.xsd\" version=\"4\">\n\
\t<Item class=\"UserGameSettings\" referent=\"RBX0\">\n\
\t\t<Properties>\n\
\t\t\t<bool name=\"Fullscreen\">false</bool>\n\
\t\t\t<int name=\"FramerateCap\">60</int>\n\
\t\t\t<int name=\"GraphicsQualityLevel\">10</int>\n\
\t\t\t<float name=\"MasterVolume\">0.5</float>\n\
\t\t\t<token name=\"SavedQualityLevel\">0</token>\n\
\t\t\t<bool name=\"VREnabled\">false</bool>\n\
\t\t</Properties>\n\
\t</Item>\n\
</roblox>\n";

    // ----- defaults and effective -----

    #[test]
    fn defaults_match_spec() {
        let g = GraphicsSettings::default();
        assert!(!g.enabled);
        assert_eq!(g.fps_cap, 0);
        // Roblox's own defaults: nothing changes until the user asks.
        assert!(!g.low_graphics);
        assert!(!g.lighter_rendering);
        assert!(!g.mute_volume);
        assert!(g.custom_flags.is_empty());
        let r = g.render;
        assert_eq!(r.msaa, Msaa::X1);
        assert_eq!(r.texture, Texture::Lowest);
        assert_eq!(r.mode, RenderMode::Default);
        assert!(!r.gray_sky);
        assert!(r.no_grass);
        assert!(r.lowest_frm);
        assert!(!r.pause_voxelizer);
        let a = AccountGraphics::default();
        assert_eq!(a.fps_cap, None);
        assert!(!a.full_quality);
    }

    #[test]
    fn old_store_loads_with_defaults() {
        let g: GraphicsSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(g, GraphicsSettings::default());
        let g: GraphicsSettings = serde_json::from_str(r#"{"enabled":true,"render":{"msaa":"X4"}}"#).unwrap();
        assert!(g.enabled);
        assert_eq!(g.render.msaa, Msaa::X4);
        assert_eq!(g.render.texture, Texture::Lowest);
        let round: GraphicsSettings =
            serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(round, g);
    }

    #[test]
    fn effective_main_is_none() {
        assert_eq!(effective(&enabled(), &AccountGraphics::default(), true), None);
    }

    #[test]
    fn effective_disabled_is_none() {
        assert_eq!(
            effective(&GraphicsSettings::default(), &AccountGraphics::default(), false),
            None
        );
    }

    #[test]
    fn effective_defaults() {
        let g = GraphicsSettings {
            fps_cap: 30,
            custom_flags: map(&[("FFlagX", "True")]),
            ..enabled()
        };
        let e = effective(&g, &AccountGraphics::default(), false).unwrap();
        assert_eq!(e.fps_cap, 30);
        assert!(e.low_graphics);
        assert_eq!(e.lighter, Some(RenderFlags::default()));
        assert_eq!(e.custom, g.custom_flags);
    }

    #[test]
    fn effective_full_quality_keeps_custom_and_cap() {
        let g = GraphicsSettings {
            fps_cap: 30,
            custom_flags: map(&[("FFlagX", "True")]),
            ..enabled()
        };
        let a = AccountGraphics {
            fps_cap: None,
            full_quality: true,
        };
        let e = effective(&g, &a, false).unwrap();
        assert_eq!(e.fps_cap, 30);
        assert!(!e.low_graphics);
        assert_eq!(e.lighter, None);
        assert_eq!(e.custom, g.custom_flags);
    }

    #[test]
    fn effective_account_cap_overrides_and_clamps() {
        let g = GraphicsSettings {
            fps_cap: 30,
            ..enabled()
        };
        let cap = |c: Option<u32>| {
            effective(
                &g,
                &AccountGraphics {
                    fps_cap: c,
                    full_quality: false,
                },
                false,
            )
            .unwrap()
            .fps_cap
        };
        assert_eq!(cap(None), 30);
        assert_eq!(cap(Some(0)), 0);
        assert_eq!(cap(Some(1)), 5);
        assert_eq!(cap(Some(5)), 5);
        assert_eq!(cap(Some(144)), 144);
        assert_eq!(cap(Some(360)), 360);
        assert_eq!(cap(Some(1000)), 360);
        let g = GraphicsSettings {
            fps_cap: 9999,
            ..enabled()
        };
        assert_eq!(
            effective(&g, &AccountGraphics::default(), false).unwrap().fps_cap,
            360
        );
        let g = GraphicsSettings {
            fps_cap: 2,
            ..enabled()
        };
        assert_eq!(
            effective(&g, &AccountGraphics::default(), false).unwrap().fps_cap,
            5
        );
    }

    // ----- flag_set -----

    fn no_extras() -> RenderFlags {
        RenderFlags {
            msaa: Msaa::Default,
            texture: Texture::Default,
            mode: RenderMode::Default,
            gray_sky: false,
            no_grass: false,
            lowest_frm: false,
            pause_voxelizer: false,
        }
    }

    const LOD: [(&str, &str); 4] = [
        ("DFIntCSGLevelOfDetailSwitchingDistance", "0"),
        ("DFIntCSGLevelOfDetailSwitchingDistanceL12", "0"),
        ("DFIntCSGLevelOfDetailSwitchingDistanceL23", "0"),
        ("DFIntCSGLevelOfDetailSwitchingDistanceL34", "0"),
    ];

    fn lod_plus(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut m = map(&LOD);
        m.extend(map(extra));
        m
    }

    #[test]
    fn flag_set_lighter_off_only_custom() {
        let e = Effective {
            fps_cap: 60,
            low_graphics: true,
            lighter: None,
            custom: map(&[("FFlagUser", "1")]),
            mute: false,
        };
        assert_eq!(flag_set(&e), map(&[("FFlagUser", "1")]));
        let e = Effective {
            custom: BTreeMap::new(),
            ..e
        };
        assert!(flag_set(&e).is_empty());
    }

    #[test]
    fn flag_set_lighter_minimal_is_lod_only() {
        assert_eq!(flag_set(&lighter(no_extras())), map(&LOD));
    }

    #[test]
    fn flag_set_msaa() {
        let cases = [
            (Msaa::Default, None),
            (Msaa::X1, Some("1")),
            (Msaa::X2, Some("2")),
            (Msaa::X4, Some("4")),
        ];
        for (msaa, want) in cases {
            let r = RenderFlags {
                msaa,
                ..no_extras()
            };
            let expect = match want {
                Some(v) => lod_plus(&[("FIntDebugForceMSAASamples", v)]),
                None => map(&LOD),
            };
            assert_eq!(flag_set(&lighter(r)), expect, "{msaa:?}");
        }
    }

    #[test]
    fn flag_set_texture() {
        let cases = [
            (Texture::Default, None),
            (Texture::Lowest, Some("0")),
            (Texture::Low, Some("1")),
            (Texture::Medium, Some("2")),
            (Texture::High, Some("3")),
        ];
        for (texture, want) in cases {
            let r = RenderFlags {
                texture,
                ..no_extras()
            };
            let expect = match want {
                Some(v) => lod_plus(&[
                    ("DFFlagTextureQualityOverrideEnabled", "True"),
                    ("DFIntTextureQualityOverride", v),
                ]),
                None => map(&LOD),
            };
            assert_eq!(flag_set(&lighter(r)), expect, "{texture:?}");
        }
    }

    #[test]
    fn flag_set_render_mode() {
        let cases = [
            (RenderMode::Default, None),
            (RenderMode::D3D11, Some("FFlagDebugGraphicsPreferD3D11")),
            (RenderMode::Vulkan, Some("FFlagDebugGraphicsPreferVulkan")),
            (RenderMode::OpenGl, Some("FFlagDebugGraphicsPreferOpenGL")),
        ];
        for (mode, want) in cases {
            let r = RenderFlags {
                mode,
                ..no_extras()
            };
            let set = flag_set(&lighter(r));
            let expect = match want {
                Some(k) => lod_plus(&[(k, "True")]),
                None => map(&LOD),
            };
            assert_eq!(set, expect, "{mode:?}");
            let present: Vec<_> = RENDER_MODE_FLAGS
                .iter()
                .filter(|f| set.contains_key(**f))
                .collect();
            assert_eq!(present.len(), usize::from(want.is_some()), "{mode:?}");
        }
    }

    #[test]
    fn flag_set_bool_switches() {
        let r = RenderFlags {
            gray_sky: true,
            ..no_extras()
        };
        assert_eq!(flag_set(&lighter(r)), lod_plus(&[("FFlagDebugSkyGray", "True")]));
        let r = RenderFlags {
            no_grass: true,
            ..no_extras()
        };
        assert_eq!(
            flag_set(&lighter(r)),
            lod_plus(&[("FIntFRMMinGrassDistance", "0"), ("FIntFRMMaxGrassDistance", "0")])
        );
        let r = RenderFlags {
            lowest_frm: true,
            ..no_extras()
        };
        assert_eq!(
            flag_set(&lighter(r)),
            lod_plus(&[("DFIntDebugFRMQualityLevelOverride", "1")])
        );
        let r = RenderFlags {
            pause_voxelizer: true,
            ..no_extras()
        };
        assert_eq!(
            flag_set(&lighter(r)),
            lod_plus(&[("DFFlagDebugPauseVoxelizer", "True")])
        );
    }

    #[test]
    fn flag_set_default_render_flags() {
        let set = flag_set(&lighter(RenderFlags::default()));
        assert_eq!(
            set,
            lod_plus(&[
                ("FIntDebugForceMSAASamples", "1"),
                ("DFFlagTextureQualityOverrideEnabled", "True"),
                ("DFIntTextureQualityOverride", "0"),
                ("FIntFRMMinGrassDistance", "0"),
                ("FIntFRMMaxGrassDistance", "0"),
                ("DFIntDebugFRMQualityLevelOverride", "1"),
            ])
        );
    }

    #[test]
    fn flag_set_custom_wins() {
        let mut e = lighter(RenderFlags::default());
        e.custom = map(&[
            ("FIntDebugForceMSAASamples", "8"),
            ("FFlagUserThing", "False"),
        ]);
        let set = flag_set(&e);
        assert_eq!(set["FIntDebugForceMSAASamples"], "8");
        assert_eq!(set["FFlagUserThing"], "False");
        assert_eq!(set["DFIntCSGLevelOfDetailSwitchingDistance"], "0");
    }

    // ----- validate_custom -----

    #[test]
    fn validate_custom_rules() {
        assert!(validate_custom(&BTreeMap::new()).is_ok());
        assert!(validate_custom(&map(&[("FFlag_Ok123", "x")])).is_ok());
        assert!(validate_custom(&map(&[("", "x")])).is_err());
        assert!(validate_custom(&map(&[("Bad Name", "x")])).is_err());
        assert!(validate_custom(&map(&[("Bad-Name", "x")])).is_err());
        assert!(validate_custom(&map(&[("Bäd", "x")])).is_err());
        let long_name = "A".repeat(128);
        assert!(validate_custom(&map(&[(&long_name, "x")])).is_ok());
        let too_long = "A".repeat(129);
        assert!(validate_custom(&map(&[(&too_long, "x")])).is_err());
        let long_value = "v".repeat(1024);
        assert!(validate_custom(&map(&[("F", &long_value)])).is_ok());
        let too_long_value = "v".repeat(1025);
        assert!(validate_custom(&map(&[("F", &too_long_value)])).is_err());
        let many: BTreeMap<String, String> = (0..500).map(|i| (format!("F{i}"), "1".to_string())).collect();
        assert!(validate_custom(&many).is_ok());
        let too_many: BTreeMap<String, String> = (0..501).map(|i| (format!("F{i}"), "1".to_string())).collect();
        assert!(validate_custom(&too_many).is_err());
    }

    // ----- merge_flags -----

    #[test]
    fn merge_blank_is_empty_object() {
        assert_eq!(merge_flags("", &BTreeMap::new()).unwrap(), "{}\n");
        assert_eq!(merge_flags("  \n", &BTreeMap::new()).unwrap(), "{}\n");
        let out = merge_flags("", &map(&[("B", "2"), ("A", "1")])).unwrap();
        assert_eq!(out, "{\n  \"A\": \"1\",\n  \"B\": \"2\"\n}\n");
    }

    #[test]
    fn merge_keeps_user_keys_and_overrides() {
        let existing = r#"{"FFlagUserKeep": "True", "FIntDebugForceMSAASamples": "8", "DFIntNumber": 42}"#;
        let out = merge_flags(existing, &map(&[("FIntDebugForceMSAASamples", "1")])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["FFlagUserKeep"], "True");
        assert_eq!(v["FIntDebugForceMSAASamples"], "1");
        assert_eq!(v["DFIntNumber"], 42);
        assert!(out.ends_with("}\n"));
        assert!(out.contains("\n  \"DFIntNumber\": 42,\n"));
    }

    #[test]
    fn merge_strips_other_render_modes() {
        let existing = r#"{"FFlagDebugGraphicsPreferVulkan": "True", "FFlagDebugGraphicsPreferOpenGL": "True", "FFlagOther": "1"}"#;
        let out = merge_flags(existing, &map(&[("FFlagDebugGraphicsPreferD3D11", "True")])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["FFlagDebugGraphicsPreferD3D11"], "True");
        assert!(v.get("FFlagDebugGraphicsPreferVulkan").is_none());
        assert!(v.get("FFlagDebugGraphicsPreferOpenGL").is_none());
        assert_eq!(v["FFlagOther"], "1");
        // no render mode in the set: existing ones are kept
        let out = merge_flags(existing, &map(&[("FFlagX", "1")])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["FFlagDebugGraphicsPreferVulkan"], "True");
        assert_eq!(v["FFlagDebugGraphicsPreferOpenGL"], "True");
    }

    #[test]
    fn merge_rejects_non_object() {
        assert!(merge_flags("[1, 2]", &BTreeMap::new()).is_err());
        assert!(merge_flags("\"str\"", &BTreeMap::new()).is_err());
        assert!(merge_flags("{not json", &BTreeMap::new()).is_err());
    }

    // ----- FlagFile -----

    #[test]
    fn flag_apply_put_back_with_existing_file() {
        let dir = TempDir::new("flags-existing");
        let path = dir.path("ClientAppSettings.json");
        let original = "{\"FFlagMine\": \"True\"}";
        fs::write(&path, original).unwrap();
        let file = FlagFile::new(&path);
        let undo = file.apply(&map(&[("FFlagRam", "1")])).unwrap();
        assert_eq!(undo.path, path);
        let written = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(v["FFlagMine"], "True");
        assert_eq!(v["FFlagRam"], "1");
        let backup = fs::read(file.backup_path()).unwrap();
        assert_eq!(backup[0], 1);
        assert_eq!(&backup[1..], original.as_bytes());
        undo.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(!file.backup_path().exists());
        assert!(!with_suffix(&path, ".ram-tmp").exists());
        // idempotent
        file.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn flag_apply_put_back_without_existing_file() {
        let dir = TempDir::new("flags-missing");
        let path = dir.path("sub").join("ClientAppSettings.json");
        let file = FlagFile::new(&path);
        file.apply(&map(&[("FFlagRam", "1")])).unwrap();
        assert!(path.exists());
        assert_eq!(fs::read(file.backup_path()).unwrap(), vec![0u8]);
        file.put_back().unwrap();
        assert!(!path.exists());
        assert!(!file.backup_path().exists());
        file.put_back().unwrap();
    }

    #[test]
    fn flag_apply_restores_leftover_backup_first() {
        let dir = TempDir::new("flags-leftover");
        let path = dir.path("ClientAppSettings.json");
        let file = FlagFile::new(&path);
        // simulate a crash: backup says the original was {"FFlagOrig":"1"},
        // the flag file still holds our merged content
        let mut backup = vec![1u8];
        backup.extend_from_slice(b"{\"FFlagOrig\": \"1\"}");
        fs::write(file.backup_path(), &backup).unwrap();
        fs::write(&path, "{\"FFlagOrig\": \"1\", \"FFlagStale\": \"x\"}").unwrap();
        file.apply(&map(&[("FFlagNew", "2")])).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["FFlagOrig"], "1");
        assert_eq!(v["FFlagNew"], "2");
        assert!(v.get("FFlagStale").is_none(), "stale content must be dropped");
        let backup = fs::read(file.backup_path()).unwrap();
        assert_eq!(&backup[1..], b"{\"FFlagOrig\": \"1\"}");
        file.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"FFlagOrig\": \"1\"}");
    }

    #[test]
    fn flag_apply_leftover_zero_backup_deletes_stale_file() {
        let dir = TempDir::new("flags-leftover0");
        let path = dir.path("ClientAppSettings.json");
        let file = FlagFile::new(&path);
        fs::write(file.backup_path(), [0u8]).unwrap();
        fs::write(&path, "{\"FFlagStale\": \"x\"}").unwrap();
        file.apply(&map(&[("FFlagNew", "2")])).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"FFlagNew\": \"2\"\n}\n");
        assert_eq!(fs::read(file.backup_path()).unwrap(), vec![0u8]);
    }

    #[test]
    fn flag_apply_refuses_non_object_and_writes_nothing() {
        let dir = TempDir::new("flags-bad");
        let path = dir.path("ClientAppSettings.json");
        fs::write(&path, "[1,2,3]").unwrap();
        let file = FlagFile::new(&path);
        assert!(file.apply(&map(&[("FFlagRam", "1")])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[1,2,3]");
        assert!(!file.backup_path().exists());
    }

    #[test]
    fn restore_leftovers_handles_many() {
        let dir = TempDir::new("flags-leftovers");
        let a = dir.path("a.json");
        let b = dir.path("b").join("b.json");
        let c = dir.path("c.json");
        fs::write(&a, "{}").unwrap();
        let fa = FlagFile::new(&a);
        fa.apply(&map(&[("F", "1")])).unwrap();
        let fb = FlagFile::new(&b);
        fb.apply(&map(&[("F", "1")])).unwrap();
        let errors = restore_leftovers(&[a.clone(), b.clone(), c.clone()]);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(fs::read_to_string(&a).unwrap(), "{}");
        assert!(!b.exists());
        assert!(!c.exists());
        assert!(!fa.backup_path().exists());
        assert!(!fb.backup_path().exists());
    }

    // ----- XML -----

    #[test]
    fn xml_get_sample() {
        let v = xml_get(SAMPLE_XML);
        assert_eq!(v.framerate_cap, Some(60));
        assert_eq!(v.quality_level, Some(10));
        assert_eq!(v.saved_quality, Some(0));
        assert_eq!(xml_get(EMPTY_XML), XmlValues::default());
        assert_eq!(xml_get("<int name=\"FramerateCap\"> 144 </int>").framerate_cap, Some(144));
        assert_eq!(xml_get("<int name=\"FramerateCap\">abc</int>").framerate_cap, None);
        assert_eq!(xml_get("<int name=\"FramerateCap\">144").framerate_cap, None);
    }

    #[test]
    fn xml_set_replaces_in_place() {
        let out = xml_set(
            SAMPLE_XML,
            &XmlValues {
                framerate_cap: Some(30),
                quality_level: Some(1),
                saved_quality: Some(1),
            master_volume: None,
        },
        );
        let expect = SAMPLE_XML
            .replace("<int name=\"FramerateCap\">60</int>", "<int name=\"FramerateCap\">30</int>")
            .replace(
                "<int name=\"GraphicsQualityLevel\">10</int>",
                "<int name=\"GraphicsQualityLevel\">1</int>",
            )
            .replace(
                "<token name=\"SavedQualityLevel\">0</token>",
                "<token name=\"SavedQualityLevel\">1</token>",
            );
        assert_eq!(out, expect);
        // None leaves everything byte-identical
        assert_eq!(xml_set(SAMPLE_XML, &XmlValues::default()), SAMPLE_XML);
        let only_cap = xml_set(
            SAMPLE_XML,
            &XmlValues {
                framerate_cap: Some(144),
                ..XmlValues::default()
            },
        );
        assert!(only_cap.contains("<int name=\"FramerateCap\">144</int>"));
        assert!(only_cap.contains("<int name=\"GraphicsQualityLevel\">10</int>"));
        assert!(only_cap.contains("<token name=\"SavedQualityLevel\">0</token>"));
    }

    #[test]
    fn xml_set_inserts_when_missing() {
        let out = xml_set(
            EMPTY_XML,
            &XmlValues {
                framerate_cap: Some(30),
                quality_level: Some(1),
                saved_quality: Some(1),
            master_volume: None,
        },
        );
        let expect = EMPTY_XML.replace(
            "\t\t<Properties>\n\t\t</Properties>",
            "\t\t<Properties>\n\t\t\t<int name=\"FramerateCap\">30</int>\n\t\t\t<int name=\"GraphicsQualityLevel\">1</int>\n\t\t\t<token name=\"SavedQualityLevel\">1</token>\n\t\t</Properties>",
        );
        assert_eq!(out, expect);
        let v = xml_get(&out);
        assert_eq!(v.framerate_cap, Some(30));
        assert_eq!(v.quality_level, Some(1));
        assert_eq!(v.saved_quality, Some(1));
        // inserted next to other properties, keeping them
        let without_cap = SAMPLE_XML.replace("\t\t\t<int name=\"FramerateCap\">60</int>\n", "");
        let out = xml_set(
            &without_cap,
            &XmlValues {
                framerate_cap: Some(45),
                ..XmlValues::default()
            },
        );
        assert_eq!(
            out,
            without_cap.replace(
                "\t\t</Properties>",
                "\t\t\t<int name=\"FramerateCap\">45</int>\n\t\t</Properties>"
            )
        );
        assert!(out.contains("<bool name=\"Fullscreen\">false</bool>"));
        // one-line Properties
        let one_line = "<roblox>\n\t<Item>\n\t\t<Properties></Properties>\n\t</Item>\n</roblox>";
        let out = xml_set(
            one_line,
            &XmlValues {
                framerate_cap: Some(5),
                ..XmlValues::default()
            },
        );
        assert_eq!(xml_get(&out).framerate_cap, Some(5));
        assert!(out.contains("<Properties>\n\t\t\t<int name=\"FramerateCap\">5</int>\n\t\t</Properties>"));
    }

    #[test]
    fn xml_set_replaces_half_written_file() {
        let half = &SAMPLE_XML[..SAMPLE_XML.len() / 2];
        let want = XmlValues {
            framerate_cap: Some(30),
            ..XmlValues::default()
        };
        let out = xml_set(half, &want);
        assert_eq!(out, xml_set(EMPTY_XML, &want));
        assert!(!out.contains("Fullscreen"));
        // no </Properties> at all
        let out = xml_set("<roblox></roblox>", &want);
        assert_eq!(out, xml_set(EMPTY_XML, &want));
        // empty / garbage
        assert_eq!(xml_set("", &want), xml_set(EMPTY_XML, &want));
        // trailing whitespace after </roblox> is fine
        let padded = format!("{SAMPLE_XML}\r\n\r\n");
        assert!(xml_set(&padded, &want).contains("Fullscreen"));
    }

    #[test]
    fn xml_for_launch_values() {
        let current = XmlValues {
            framerate_cap: Some(60),
            quality_level: Some(10),
            saved_quality: Some(0),
        master_volume: None,
    };
        let e = Effective {
            fps_cap: 30,
            low_graphics: true,
            lighter: None,
            custom: BTreeMap::new(),
            mute: false,
        };
        assert_eq!(
            xml_for_launch(&e, &current),
            XmlValues {
                framerate_cap: Some(30),
                quality_level: Some(1),
                saved_quality: Some(1),
            master_volume: None,
        }
        );
        let e = Effective {
            fps_cap: 0,
            low_graphics: false,
            lighter: None,
            custom: BTreeMap::new(),
            mute: false,
        };
        assert_eq!(xml_for_launch(&e, &current), current);
        assert_eq!(xml_for_launch(&e, &XmlValues::default()), XmlValues::default());
    }

    #[test]
    fn xml_file_read_missing_is_empty() {
        let dir = TempDir::new("xml-missing");
        let (text, v) = XmlFile::new(dir.path("GlobalBasicSettings_13.xml")).read();
        assert_eq!(text, EMPTY_XML);
        assert_eq!(v, XmlValues::default());
    }

    #[test]
    fn xml_apply_and_put_back_round_trip() {
        let dir = TempDir::new("xml-roundtrip");
        let path = dir.path("GlobalBasicSettings_13.xml");
        fs::write(&path, SAMPLE_XML).unwrap();
        let file = XmlFile::new(&path);
        let want = XmlValues {
            framerate_cap: Some(30),
            quality_level: Some(1),
            saved_quality: Some(1),
        master_volume: None,
    };
        let undo = file.apply(&want).unwrap();
        assert_eq!(undo.path, path);
        assert_eq!(
            undo.user,
            XmlValues {
                framerate_cap: Some(60),
                quality_level: Some(10),
                saved_quality: Some(0),
                master_volume: Some(500),
            }
        );
        assert_eq!(undo.ours, want);
        // The sample file's own volume (0.5) is untouched by this launch.
        assert_eq!(xml_get(&fs::read_to_string(&path).unwrap()), XmlValues { master_volume: Some(500), ..want.clone() });
        assert!(!with_suffix(&path, ".ram-tmp").exists());
        // persistable
        let json = serde_json::to_string(&undo).unwrap();
        let undo2: XmlUndo = serde_json::from_str(&json).unwrap();
        assert_eq!(undo2, undo);
        undo2.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE_XML);
        // again: nothing to do, still identical
        undo.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE_XML);
    }

    #[test]
    fn xml_apply_creates_file_and_put_back_removes_inserted() {
        let dir = TempDir::new("xml-create");
        let path = dir.path("Roblox").join("GlobalBasicSettings_13.xml");
        let file = XmlFile::new(&path);
        let want = XmlValues {
            framerate_cap: Some(30),
            quality_level: Some(1),
            saved_quality: Some(1),
        master_volume: None,
    };
        let undo = file.apply(&want).unwrap();
        assert_eq!(undo.user, XmlValues::default());
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(xml_get(&text), want);
        undo.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), EMPTY_XML);
    }

    #[test]
    fn xml_apply_does_not_rewrite_identical() {
        let dir = TempDir::new("xml-same");
        let path = dir.path("GlobalBasicSettings_13.xml");
        fs::write(&path, SAMPLE_XML).unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let undo = XmlFile::new(&path)
            .apply(&XmlValues {
                framerate_cap: Some(60),
                ..XmlValues::default()
            })
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert_eq!(undo.user.framerate_cap, Some(60));
        undo.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE_XML);
    }

    #[test]
    fn xml_put_back_keeps_user_edits() {
        let dir = TempDir::new("xml-user-edit");
        let path = dir.path("GlobalBasicSettings_13.xml");
        fs::write(&path, SAMPLE_XML).unwrap();
        let want = XmlValues {
            framerate_cap: Some(30),
            quality_level: Some(1),
            saved_quality: Some(1),
        master_volume: None,
    };
        let undo = XmlFile::new(&path).apply(&want).unwrap();
        // the client (or the user) changed the cap and the quality level,
        // SavedQualityLevel still holds ours
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("<int name=\"FramerateCap\">30</int>", "<int name=\"FramerateCap\">120</int>")
            .replace(
                "<int name=\"GraphicsQualityLevel\">1</int>",
                "<int name=\"GraphicsQualityLevel\">7</int>",
            );
        fs::write(&path, &edited).unwrap();
        undo.put_back().unwrap();
        let after = xml_get(&fs::read_to_string(&path).unwrap());
        assert_eq!(after.framerate_cap, Some(120));
        assert_eq!(after.quality_level, Some(7));
        assert_eq!(after.saved_quality, Some(0));
        assert!(fs::read_to_string(&path).unwrap().contains("<bool name=\"Fullscreen\">false</bool>"));
    }

    #[test]
    fn xml_put_back_only_touches_keys_we_set() {
        let dir = TempDir::new("xml-partial");
        let path = dir.path("GlobalBasicSettings_13.xml");
        fs::write(&path, SAMPLE_XML).unwrap();
        let undo = XmlFile::new(&path)
            .apply(&XmlValues {
                framerate_cap: Some(30),
                ..XmlValues::default()
            })
            .unwrap();
        // the user meanwhile set the quality to 1 themselves (equals what
        // "ours" would be if we had set it; we did not, so it must stay)
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace(
                "<int name=\"GraphicsQualityLevel\">10</int>",
                "<int name=\"GraphicsQualityLevel\">1</int>",
            );
        fs::write(&path, &edited).unwrap();
        undo.put_back().unwrap();
        let after = xml_get(&fs::read_to_string(&path).unwrap());
        assert_eq!(after.framerate_cap, Some(60));
        assert_eq!(after.quality_level, Some(1));
    }

    #[test]
    fn xml_put_back_missing_file_is_ok() {
        let dir = TempDir::new("xml-gone");
        let path = dir.path("GlobalBasicSettings_13.xml");
        let undo = XmlUndo {
            path: path.clone(),
            user: XmlValues::default(),
            ours: XmlValues {
                framerate_cap: Some(30),
                ..XmlValues::default()
            },
        };
        undo.put_back().unwrap();
        assert!(!path.exists());
        assert!(!settle(&path, &undo).unwrap());
    }

    #[test]
    fn settle_after_client_resave() {
        let dir = TempDir::new("xml-settle");
        let path = dir.path("GlobalBasicSettings_13.xml");
        fs::write(&path, SAMPLE_XML).unwrap();
        let want = XmlValues {
            framerate_cap: Some(30),
            quality_level: Some(1),
            saved_quality: Some(1),
        master_volume: None,
    };
        let undo = XmlFile::new(&path).apply(&want).unwrap();
        undo.put_back().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE_XML);
        assert!(!settle(&path, &undo).unwrap());
        // the client exits and re-saves the whole file with our values leaked
        let leaked = xml_set(SAMPLE_XML, &want).replace(
            "<bool name=\"VREnabled\">false</bool>",
            "<bool name=\"VREnabled\">false</bool>\n\t\t\t<int name=\"NewSetting\">3</int>",
        );
        fs::write(&path, &leaked).unwrap();
        assert!(settle(&path, &undo).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        let after = xml_get(&text);
        assert_eq!(after.framerate_cap, Some(60));
        assert_eq!(after.quality_level, Some(10));
        assert_eq!(after.saved_quality, Some(0));
        assert!(text.contains("<int name=\"NewSetting\">3</int>"), "client additions kept");
        assert!(!settle(&path, &undo).unwrap());
        // half-written file: error, nothing touched
        let half = &leaked[..leaked.len() / 2];
        fs::write(&path, half).unwrap();
        assert!(settle(&path, &undo).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), half);
    }

    #[test]
    fn settle_removes_values_the_user_never_had() {
        let dir = TempDir::new("xml-settle-remove");
        let path = dir.path("GlobalBasicSettings_13.xml");
        let without = SAMPLE_XML
            .replace("\t\t\t<int name=\"FramerateCap\">60</int>\n", "")
            .replace("\t\t\t<token name=\"SavedQualityLevel\">0</token>\n", "");
        fs::write(&path, &without).unwrap();
        let want = XmlValues {
            framerate_cap: Some(30),
            quality_level: Some(1),
            saved_quality: Some(1),
        master_volume: None,
    };
        let undo = XmlFile::new(&path).apply(&want).unwrap();
        assert!(settle(&path, &undo).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), without);
    }

    // ----- log detection -----

    #[test]
    fn volume_is_written_as_a_float_and_restored() {
        let dir = TempDir::new("volume");
        let path = dir.path("GlobalBasicSettings_13.xml");
        let with_volume = EMPTY_XML.replace("\t\t</Properties>", "\t\t\t<float name=\"MasterVolume\">0.75</float>\n\t\t</Properties>");
        fs::write(&path, with_volume).unwrap();
        let file = XmlFile::new(&path);
        let (_, before) = file.read();
        assert_eq!(before.master_volume, Some(750));
        let g = GraphicsSettings { mute_volume: true, ..enabled() };
        let e = effective(&g, &AccountGraphics::default(), false).unwrap();
        assert!(e.mute);
        let undo = file.apply(&xml_for_launch(&e, &before)).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("<float name=\"MasterVolume\">0</float>"));
        undo.put_back().unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("<float name=\"MasterVolume\">0.75</float>"));
        // Full quality keeps its sound.
        let fq = effective(&g, &AccountGraphics { full_quality: true, ..Default::default() }, false).unwrap();
        assert!(!fq.mute);
        assert_eq!(milli_text(1000), "1");
        assert_eq!(milli_text(500), "0.5");
        assert_eq!(milli_text(0), "0");
    }

    #[test]
    fn read_marks() {
        assert!(!has_read_mark(""));
        assert!(!has_read_mark("2026-11-21 [FLog::Output] hello"));
        assert!(has_read_mark("x\n[FLog::Graphics] SceneManager: resizing main targets 1920x1080\n"));
        assert!(has_read_mark("[FLog::Network] Replicator created: 0x1234"));
        assert!(has_read_mark("[FLog::ClientAppSettings] LoadClientSettingsFromLocal: ok"));
    }

    #[test]
    fn tracker_names() {
        let head = "... BrowserTrackerId=12345 ...";
        assert!(names_tracker(head, "12345"));
        assert!(!names_tracker(head, "1234"));
        assert!(!names_tracker(head, "123456"));
        assert!(!names_tracker(head, ""));
        assert!(names_tracker("websiteBTId is 999\n", "999"));
        assert!(!names_tracker("websiteBTId is 9991\n", "999"));
        assert!(names_tracker("BTID is overriden to 42", "42"));
        assert!(!names_tracker("BTID is overriden to 420", "42"));
        assert!(names_tracker("BrowserTrackerId=420 then BrowserTrackerId=42,", "42"));
        assert!(!names_tracker("BrowserTrackerId=42", "43"));
        assert!(!names_tracker("", "42"));
    }
}
