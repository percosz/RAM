//! Opens the user's default (or any installed Chromium-family) browser in a
//! per-account profile, already signed in to Roblox, without an embedded
//! webview.
//!
//! The browser is started with `--remote-debugging-port=0` on a private
//! `--user-data-dir`; the session cookie is handed over through the
//! Chrome DevTools Protocol over a minimal WebSocket client implemented here
//! on top of `std::net::TcpStream`. Nothing in this module reads another
//! process's memory, and the cookie value is never logged or placed in an
//! error string.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

use crate::util::{from_wide, wide};

const PORT_FILE: &str = "DevToolsActivePort";
const START_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// A listening loopback port accepts within a few milliseconds.
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(100);
/// Upper bound on one CDP message; `Storage.getCookies` on a busy profile is
/// well under a megabyte.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

const CHROMIUM_STEMS: &[&str] = &[
    "brave", "chrome", "msedge", "chromium", "vivaldi", "opera", "opera_gx",
];

/// A browser executable and what we know about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browser {
    pub exe: PathBuf,
    /// "Brave", "Chrome", "Edge", "Chromium", "Vivaldi", "Opera", "Firefox",
    /// or the executable's file stem when it is none of those.
    pub name: String,
    /// Whether the browser speaks the Chromium command line and DevTools
    /// protocol this module relies on.
    pub chromium: bool,
}

impl Browser {
    fn from_exe(exe: PathBuf) -> Self {
        let stem = exe
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let name = match stem.as_str() {
            "brave" => "Brave",
            "chrome" => "Chrome",
            "msedge" => "Edge",
            "chromium" => "Chromium",
            "vivaldi" => "Vivaldi",
            "opera" | "opera_gx" => "Opera",
            "firefox" => "Firefox",
            other => other,
        }
        .to_string();
        let chromium = CHROMIUM_STEMS.contains(&stem.as_str());
        Browser {
            exe,
            name,
            chromium,
        }
    }
}

/// A browser window opened by [`open_signed_in`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// The browser process that owns the profile (not necessarily the
    /// process we spawned: Chromium hands a second launch of a running
    /// profile to the live instance).
    pub pid: u32,
    pub profile_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Browser discovery
// ---------------------------------------------------------------------------

fn reg_sz(root: HKEY, subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    let value_ptr = value.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr()));
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS || bytes == 0 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            value_ptr,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut bytes),
        )
    };
    (status == ERROR_SUCCESS).then(|| from_wide(&buf))
}

/// The executable named by a `shell\open\command` string such as
/// `"C:\...\brave.exe" --single-argument %1` or `C:\x\y.exe -- "%1"`.
fn parse_command_exe(command: &str) -> Option<PathBuf> {
    let s = command.trim_start();
    if s.is_empty() {
        return None;
    }
    let exe = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        s.split_whitespace().next()?
    };
    let exe = exe.trim();
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

/// The browser registered as the user's `https` handler.
pub fn default_browser() -> Option<Browser> {
    // Newer Windows 11 builds record the choice under `UserChoiceLatest` and
    // can leave the older `UserChoice` key pointing at the previous browser,
    // so the newer key wins when it exists.
    const ASSOC: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https";
    let prog_id = reg_sz(HKEY_CURRENT_USER, &format!(r"{ASSOC}\UserChoiceLatest\ProgId"), Some("ProgId"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| reg_sz(HKEY_CURRENT_USER, &format!(r"{ASSOC}\UserChoice"), Some("ProgId")))?;
    let prog_id = prog_id.trim();
    if prog_id.is_empty() {
        return None;
    }
    let command = reg_sz(
        HKEY_CLASSES_ROOT,
        &format!(r"{prog_id}\shell\open\command"),
        None,
    )?;
    let exe = parse_command_exe(&command)?;
    Some(Browser::from_exe(exe))
}

/// Chromium-family browsers found at their usual install locations, so the
/// app can fall back to one when the default browser is Firefox.
pub fn known_chromium_browsers() -> Vec<Browser> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let local = env("LOCALAPPDATA");
    let pf = env("ProgramFiles");
    let pf86 = env("ProgramFiles(x86)");
    let candidates: [(Option<&PathBuf>, &str); 8] = [
        (
            local.as_ref(),
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
        (
            pf.as_ref(),
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
        (pf.as_ref(), r"Google\Chrome\Application\chrome.exe"),
        (pf86.as_ref(), r"Google\Chrome\Application\chrome.exe"),
        (local.as_ref(), r"Google\Chrome\Application\chrome.exe"),
        (pf86.as_ref(), r"Microsoft\Edge\Application\msedge.exe"),
        (pf.as_ref(), r"Microsoft\Edge\Application\msedge.exe"),
        (local.as_ref(), r"Vivaldi\Application\vivaldi.exe"),
    ];
    let mut out: Vec<Browser> = Vec::new();
    for (root, rel) in candidates {
        let Some(root) = root else { continue };
        let exe = root.join(rel);
        if exe.is_file() && !out.iter().any(|b| b.exe == exe) {
            out.push(Browser::from_exe(exe));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// DevToolsActivePort
// ---------------------------------------------------------------------------

/// Parses the two-line `DevToolsActivePort` file: port, then the
/// `/devtools/browser/<uuid>` path.
fn parse_port_file(text: &str) -> Option<(u16, String)> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let port: u16 = lines.next()?.parse().ok()?;
    let path = lines.next()?;
    if port == 0 || !path.starts_with('/') {
        return None;
    }
    Some((port, path.to_string()))
}

fn read_port_file(profile_dir: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(profile_dir.join(PORT_FILE)).ok()?;
    parse_port_file(&text)
}

// ---------------------------------------------------------------------------
// Base64 (for the WebSocket key)
// ---------------------------------------------------------------------------

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------------
// WebSocket framing
// ---------------------------------------------------------------------------

const OP_CONT: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

#[derive(Debug, PartialEq, Eq)]
struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

/// Encodes one frame. Client-to-server frames carry a mask.
fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push((if fin { 0x80 } else { 0 }) | (opcode & 0x0F));
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    let len = payload.len();
    if len < 126 {
        out.push(mask_bit | len as u8);
    } else if len <= 0xFFFF {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

fn read_exact<R: Read>(r: &mut R, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Decodes one frame from `r`, unmasking it if the mask bit is set.
fn read_frame<R: Read>(r: &mut R) -> io::Result<Frame> {
    let head = read_exact(r, 2)?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let b = read_exact(r, 2)?;
            u64::from(u16::from_be_bytes([b[0], b[1]]))
        }
        127 => {
            let b = read_exact(r, 8)?;
            u64::from_be_bytes(b.try_into().expect("8 bytes"))
        }
        n => u64::from(n),
    };
    if len > MAX_MESSAGE as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WebSocket frame too large",
        ));
    }
    let mask = if masked {
        let m = read_exact(r, 4)?;
        Some([m[0], m[1], m[2], m[3]])
    } else {
        None
    };
    let mut payload = read_exact(r, len as usize)?;
    if let Some(key) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }
    Ok(Frame {
        fin,
        opcode,
        payload,
    })
}

/// A `Read` over a `TcpStream` that first yields bytes left over from the
/// HTTP upgrade response.
struct Prefixed {
    pending: Vec<u8>,
    at: usize,
    stream: TcpStream,
}

impl Read for Prefixed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at < self.pending.len() {
            let n = (self.pending.len() - self.at).min(buf.len());
            buf[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
            self.at += n;
            if self.at == self.pending.len() {
                self.pending.clear();
                self.at = 0;
            }
            return Ok(n);
        }
        self.stream.read(buf)
    }
}

// ---------------------------------------------------------------------------
// CDP client
// ---------------------------------------------------------------------------

struct Cdp {
    reader: Prefixed,
    writer: TcpStream,
    next_id: u64,
}

impl Cdp {
    /// Connects to the DevTools endpoint named by the profile's port file.
    fn connect(profile_dir: &Path) -> Result<Self, String> {
        let (port, path) = read_port_file(profile_dir).ok_or("no DevTools port file")?;
        Self::connect_to(port, &path)
    }

    fn connect_to(port: u16, path: &str) -> Result<Self, String> {
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| format!("connect to 127.0.0.1:{port}: {e}"))?;
        stream
            .set_read_timeout(Some(CALL_TIMEOUT))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(CALL_TIMEOUT))
            .map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);

        let key = base64(&crate::crypto::random_bytes(16));
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("WebSocket handshake write: {e}"))?;

        // Read until the end of the response headers; keep any frame bytes
        // that arrived in the same read.
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            if let Some(i) = find_header_end(&buf) {
                break i;
            }
            if buf.len() > 64 * 1024 {
                return Err("WebSocket handshake: response too long".into());
            }
            let n = stream
                .read(&mut chunk)
                .map_err(|e| format!("WebSocket handshake read: {e}"))?;
            if n == 0 {
                return Err("WebSocket handshake: connection closed".into());
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let status = head.lines().next().unwrap_or("");
        let code = status.split_whitespace().nth(1).unwrap_or("");
        if code != "101" {
            let hint = if code == "403" {
                " (the browser rejected the origin; --remote-allow-origins would be needed)"
            } else {
                ""
            };
            return Err(format!("WebSocket handshake refused: {status}{hint}"));
        }
        let pending = buf[header_end + 4..].to_vec();
        let writer = stream
            .try_clone()
            .map_err(|e| format!("socket clone: {e}"))?;
        Ok(Cdp {
            reader: Prefixed {
                pending,
                at: 0,
                stream,
            },
            writer,
            next_id: 0,
        })
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let rnd = crate::crypto::random_bytes(4);
        let mask = [rnd[0], rnd[1], rnd[2], rnd[3]];
        let frame = encode_frame(true, opcode, payload, Some(mask));
        self.writer
            .write_all(&frame)
            .map_err(|e| format!("WebSocket write: {e}"))
    }

    /// Next complete text message, answering pings and reporting a close.
    fn recv_message(&mut self) -> Result<String, String> {
        let mut assembled: Vec<u8> = Vec::new();
        let mut in_message = false;
        loop {
            let frame = read_frame(&mut self.reader).map_err(|e| {
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
                    "the browser did not answer within 10 s".to_string()
                } else {
                    format!("WebSocket read: {e}")
                }
            })?;
            match frame.opcode {
                OP_PING => self.send_frame(OP_PONG, &frame.payload)?,
                OP_PONG => {}
                OP_CLOSE => {
                    let _ = self.send_frame(OP_CLOSE, &[]);
                    return Err("the browser closed the DevTools connection".into());
                }
                OP_TEXT | OP_CONT | 0x2 => {
                    if frame.opcode != OP_CONT {
                        assembled.clear();
                        in_message = true;
                    } else if !in_message {
                        continue;
                    }
                    if assembled.len() + frame.payload.len() > MAX_MESSAGE {
                        return Err("DevTools message too large".into());
                    }
                    assembled.extend_from_slice(&frame.payload);
                    if frame.fin {
                        return String::from_utf8(std::mem::take(&mut assembled))
                            .map_err(|_| "DevTools message is not UTF-8".to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// Sends one CDP command and returns its `result`, or `Err` with the
    /// protocol error text. Events and other replies are skipped.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call_in(None, method, params)
    }

    /// Like `call`, addressed to an attached target session (flat mode)
    /// when `session` is given.
    fn call_in(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut envelope = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            envelope["sessionId"] = json!(s);
        }
        let msg = envelope.to_string();
        self.send_frame(OP_TEXT, msg.as_bytes())?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return Err(format!("{method}: no reply within 10 s"));
            }
            let text = self.recv_message()?;
            let v: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                let m = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("{method}: {m}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn close(mut self) {
        let _ = self.send_frame(OP_CLOSE, &[0x03, 0xE8]);
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

// ---------------------------------------------------------------------------
// Public operations
// ---------------------------------------------------------------------------

/// Spawns `browser` on `profile_dir`, seeds the Roblox session cookie when
/// given, and opens `url` in a tab. The cookie value never appears in the
/// returned error.
///
/// `window_title_hint` is accepted for callers that want to label the
/// window; Chromium offers no way to set a window title over DevTools, so it
/// is currently unused.
pub fn open_signed_in(
    browser: &Browser,
    profile_dir: &Path,
    cookie: Option<&str>,
    url: &str,
    _window_title_hint: &str,
) -> Result<Session, String> {
    let name = browser.name.as_str();
    if !browser.chromium {
        return Err(format!(
            "{name} is not a Chromium-based browser; RAM can only sign in Brave, Chrome, Edge, Chromium, Vivaldi or Opera"
        ));
    }
    std::fs::create_dir_all(profile_dir)
        .map_err(|e| format!("could not create the browser profile folder: {e}"))?;

    // A live instance on this profile answers on its existing port; a stale
    // port file from a crash must go before the new process writes its own.
    let (mut cdp, mut child) = match Cdp::connect(profile_dir) {
        Ok(cdp) => (cdp, None),
        Err(_) => {
            let _ = std::fs::remove_file(profile_dir.join(PORT_FILE));
            let mut cmd = Command::new(&browser.exe);
            cmd.arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-sync")
                .arg("--remote-debugging-port=0")
                .arg("--new-window")
                .arg("about:blank")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let child = cmd.spawn().map_err(|_| format!("{name} did not start"))?;
            let cdp = wait_for_devtools(profile_dir, name, &child)?;
            (cdp, Some(child))
        }
    };

    let result: Result<Option<u32>, String> = (|| {
        if let Some(value) = cookie {
            cdp.call(
                "Storage.setCookies",
                json!({ "cookies": [{
                    "name": ".ROBLOSECURITY",
                    "value": value,
                    "domain": ".roblox.com",
                    "path": "/",
                    "secure": true,
                    "httpOnly": true,
                    "sameSite": "Lax",
                }]}),
            )
            .map_err(|_| format!("{name} refused the session cookie"))?;
        }
        let created = cdp.call(
            "Target.createTarget",
            json!({ "url": url, "newWindow": false }),
        )?;
        let created_id = created
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // Drop the launcher's about:blank tab(s) now that a real one exists.
        let targets = cdp.call("Target.getTargets", json!({}))?;
        if let Some(list) = targets.get("targetInfos").and_then(Value::as_array) {
            for t in list {
                let is_page = t.get("type").and_then(Value::as_str) == Some("page");
                let blank = t.get("url").and_then(Value::as_str) == Some("about:blank");
                let id = t.get("targetId").and_then(Value::as_str).unwrap_or("");
                if is_page && blank && id != created_id {
                    let _ = cdp.call("Target.closeTarget", json!({ "targetId": id }));
                }
            }
        }
        let pid = browser_pid(&mut cdp).or_else(|| child.as_ref().map(|c| c.id()));
        Ok(pid)
    })();

    cdp.close();
    // Not waited: the process outlives this call by design.
    let fallback_pid = child.take().map(|c| c.id());
    let pid = result?.or(fallback_pid).unwrap_or(0);
    Ok(Session {
        pid,
        profile_dir: profile_dir.to_path_buf(),
    })
}

/// Polls for the port file and a working DevTools connection.
fn wait_for_devtools(
    profile_dir: &Path,
    name: &str,
    child: &std::process::Child,
) -> Result<Cdp, String> {
    let start = Instant::now();
    let mut exited_at: Option<Instant> = None;
    loop {
        if let Some((port, path)) = read_port_file(profile_dir) {
            if let Ok(cdp) = Cdp::connect_to(port, &path) {
                return Ok(cdp);
            }
        }
        if start.elapsed() >= START_TIMEOUT {
            return Err(format!(
                "{name} did not open its DevTools port within 15 s"
            ));
        }
        // A process that exited without ever writing the port file (and
        // where no live instance answers) did not start. Allow a short grace
        // period: when handing off to a running instance, the file is
        // already there, but a freshly written one may still be settling.
        let exited = exited_at.is_some()
            || {
                // `try_wait` needs `&mut`; use a query on the pid instead.
                !process_alive(child.id())
            };
        if exited {
            let at = *exited_at.get_or_insert_with(Instant::now);
            if at.elapsed() > Duration::from_secs(2) {
                return Err(format!("{name} did not start"));
            }
        }
        std::thread::sleep(POLL);
    }
}

/// The browser (main) process id as the browser itself reports it.
fn browser_pid(cdp: &mut Cdp) -> Option<u32> {
    let info = cdp.call("SystemInfo.getProcessInfo", json!({})).ok()?;
    info.get("processInfo")?
        .as_array()?
        .iter()
        .find(|p| p.get("type").and_then(Value::as_str) == Some("browser"))?
        .get("id")?
        .as_u64()
        .map(|id| id as u32)
}

/// The `.ROBLOSECURITY` cookie currently held by the running browser on
/// `profile_dir`, or `Ok(None)` when it has none.
pub fn read_cookie(profile_dir: &Path) -> Result<Option<String>, String> {
    let mut cdp = Cdp::connect(profile_dir)
        .map_err(|_| "The browser window for this account is not open".to_string())?;
    let result = cdp.call("Storage.getCookies", json!({}));
    cdp.close();
    let cookies = result.map_err(|e| format!("could not read the browser's cookies: {e}"))?;
    let value = cookies
        .get("cookies")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter().find(|c| {
                c.get("name").and_then(Value::as_str) == Some(".ROBLOSECURITY")
                    && c.get("domain")
                        .and_then(Value::as_str)
                        .is_some_and(|d| d.ends_with("roblox.com"))
            })
        })
        .and_then(|c| c.get("value").and_then(Value::as_str))
        .map(str::to_string);
    Ok(value)
}

/// Whether a browser is serving DevTools on `profile_dir` right now.
///
/// A port file whose port no longer answers is a leftover (the browser was
/// killed, or the PC shut down with it open): it is deleted, so the next
/// check is a plain "no file" and costs nothing. The probe uses a short
/// timeout because Windows takes about two seconds to report a refused
/// loopback connection, which once made RAM's start-up wait on every
/// stale profile.
pub fn is_running(profile_dir: &Path) -> bool {
    let Some((port, _)) = read_port_file(profile_dir) else {
        return false;
    };
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    if TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(profile_dir.join("DevToolsActivePort"));
    false
}

/// Asks the running browser on `profile_dir` to quit (`Browser.close`).
pub fn close(profile_dir: &Path) -> Result<(), String> {
    let mut cdp = Cdp::connect(profile_dir)
        .map_err(|_| "The browser window for this account is not open".to_string())?;
    // The browser may drop the socket before replying; that is still success.
    let result = cdp.call("Browser.close", json!({}));
    cdp.close();
    match result {
        Ok(_) => Ok(()),
        Err(e) if e.contains("closed the DevTools connection") || e.contains("WebSocket read") => {
            Ok(())
        }
        Err(e) => Err(format!("could not close the browser: {e}")),
    }
}

fn process_alive(pid: u32) -> bool {
    !wait_exit(pid, Duration::ZERO)
}

/// Waits for process `pid` to exit. Returns `true` once it has (or if it no
/// longer exists), `false` on timeout.
pub fn wait_exit(pid: u32, timeout: Duration) -> bool {
    if pid == 0 {
        return true;
    }
    let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(h) => h,
        // ERROR_INVALID_PARAMETER: no such process any more.
        Err(_) => return true,
    };
    let ms = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
    let waited = unsafe { WaitForSingleObject(handle, ms) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    waited == WAIT_OBJECT_0
}

/// Removes every Roblox session the profile holds. When the browser is
/// running this goes through DevTools; otherwise the on-disk cookie stores
/// are deleted so the vault stays the only place a session lives.
pub fn clear_cookies(profile_dir: &Path) -> Result<(), String> {
    if is_running(profile_dir) {
        let mut cdp = Cdp::connect(profile_dir)
            .map_err(|_| "The browser window for this account is not open".to_string())?;
        let result = cdp.call("Storage.clearCookies", json!({}));
        cdp.close();
        return result
            .map(|_| ())
            .map_err(|e| format!("could not clear the browser's cookies: {e}"));
    }
    let default = profile_dir.join("Default");
    let files = [
        default.join("Network").join("Cookies"),
        default.join("Network").join("Cookies-journal"),
        default.join("Cookies"),
        default.join("Cookies-journal"),
    ];
    for f in files {
        match std::fs::remove_file(&f) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "could not delete {}: {e}",
                    f.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ))
            }
        }
    }
    Ok(())
}

/// Deletes the whole profile folder, retrying while the browser lets go of
/// its files.
pub fn wipe_profile(profile_dir: &Path) -> Result<(), String> {
    let mut last = None;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500));
        }
        match std::fs::remove_dir_all(profile_dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(format!(
        "could not delete the browser profile: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_quoted_and_bare_commands() {
        assert_eq!(
            parse_command_exe(
                r#""C:\Users\admin\AppData\Local\BraveSoftware\Brave-Browser\Application\brave.exe" --single-argument %1"#
            ),
            Some(PathBuf::from(
                r"C:\Users\admin\AppData\Local\BraveSoftware\Brave-Browser\Application\brave.exe"
            ))
        );
        assert_eq!(
            parse_command_exe(r#"  "C:\Program Files\Google\Chrome\Application\chrome.exe" -- "%1""#),
            Some(PathBuf::from(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            ))
        );
        assert_eq!(
            parse_command_exe(r"C:\x\firefox.exe -osint -url %1"),
            Some(PathBuf::from(r"C:\x\firefox.exe"))
        );
        assert_eq!(parse_command_exe(""), None);
        assert_eq!(parse_command_exe("\"\""), None);
    }

    #[test]
    fn names_and_chromium_flag_from_stem() {
        let b = Browser::from_exe(PathBuf::from(r"C:\x\Brave.EXE"));
        assert_eq!((b.name.as_str(), b.chromium), ("Brave", true));
        let b = Browser::from_exe(PathBuf::from(r"C:\x\msedge.exe"));
        assert_eq!((b.name.as_str(), b.chromium), ("Edge", true));
        let b = Browser::from_exe(PathBuf::from(r"C:\x\opera_gx.exe"));
        assert_eq!((b.name.as_str(), b.chromium), ("Opera", true));
        let b = Browser::from_exe(PathBuf::from(r"C:\x\firefox.exe"));
        assert_eq!((b.name.as_str(), b.chromium), ("Firefox", false));
        let b = Browser::from_exe(PathBuf::from(r"C:\x\waterfox.exe"));
        assert_eq!((b.name.as_str(), b.chromium), ("waterfox", false));
    }

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // RFC 6455 example key.
        assert_eq!(
            base64(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10]),
            "AQIDBAUGBwgJCgsMDQ4PEA=="
        );
    }

    #[test]
    fn port_file_parses_two_lines() {
        assert_eq!(
            parse_port_file("51234\r\n/devtools/browser/0a1b2c3d-1111-2222-3333-444455556666\r\n"),
            Some((
                51234,
                "/devtools/browser/0a1b2c3d-1111-2222-3333-444455556666".to_string()
            ))
        );
        assert_eq!(parse_port_file("51234\n"), None);
        assert_eq!(parse_port_file("\n/devtools/browser/x"), None);
        assert_eq!(parse_port_file("abc\n/devtools/browser/x"), None);
        assert_eq!(parse_port_file("0\n/devtools/browser/x"), None);
        assert_eq!(parse_port_file("80\nnot-a-path"), None);
    }

    fn round_trip(payload: &[u8], mask: Option<[u8; 4]>, opcode: u8, fin: bool) {
        let bytes = encode_frame(fin, opcode, payload, mask);
        let mut cur = Cursor::new(bytes);
        let frame = read_frame(&mut cur).unwrap();
        assert_eq!(
            frame,
            Frame {
                fin,
                opcode,
                payload: payload.to_vec()
            }
        );
        assert_eq!(cur.position() as usize, cur.get_ref().len(), "no trailing bytes");
    }

    #[test]
    fn websocket_frames_round_trip_all_lengths() {
        let mask = Some([0x37, 0xfa, 0x21, 0x3d]);
        round_trip(b"", mask, OP_TEXT, true);
        round_trip(b"Hello", mask, OP_TEXT, true);
        round_trip(b"Hello", None, OP_TEXT, true);
        round_trip(&[0xAB; 125], mask, OP_TEXT, true);
        round_trip(&[0xCD; 126], mask, OP_TEXT, true);
        round_trip(&vec![0xEF; 65_535], mask, OP_TEXT, false);
        round_trip(&vec![0x01; 65_536], mask, 0x2, true);
        round_trip(b"ping", mask, OP_PING, true);
        round_trip(&[0x03, 0xE8], None, OP_CLOSE, true);
    }

    #[test]
    fn websocket_frame_matches_rfc6455_example() {
        // RFC 6455 section 5.7: masked "Hello".
        let bytes = encode_frame(true, OP_TEXT, b"Hello", Some([0x37, 0xfa, 0x21, 0x3d]));
        assert_eq!(
            bytes,
            [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]
        );
        let bytes = encode_frame(true, OP_TEXT, b"Hello", None);
        assert_eq!(bytes, [0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]);
        // Header lengths for 16- and 64-bit payload sizes.
        assert_eq!(encode_frame(true, 0x2, &[0u8; 256], None)[..4], [0x82, 126, 1, 0]);
        let big = encode_frame(true, 0x2, &[0u8; 65_536], None);
        assert_eq!(big[..2], [0x82, 127]);
        assert_eq!(big[2..10], [0, 0, 0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn prefixed_reader_drains_pending_then_stream() {
        // A frame split across the handshake leftover and the socket cannot
        // be simulated without a socket; check the leftover path on its own.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server.write_all(b"lo").unwrap();
        let mut r = Prefixed {
            pending: b"Hel".to_vec(),
            at: 0,
            stream: client,
        };
        let mut out = [0u8; 5];
        r.read_exact(&mut out).unwrap();
        assert_eq!(&out, b"Hello");
    }

    #[test]
    fn wait_exit_on_dead_pid_is_true() {
        // PID 0 is "unknown"; a huge PID does not exist.
        assert!(wait_exit(0, Duration::ZERO));
        assert!(wait_exit(0x7FFF_FFF0, Duration::ZERO));
    }

    /// Spawns a real Chromium-family browser on a scratch profile, seeds a
    /// fake cookie, reads it back, closes the browser and wipes the profile.
    /// Run explicitly:
    /// `cargo test -p ram-win --lib sysbrowser -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn brave_round_trip() {
        let browser = default_browser()
            .filter(|b| b.chromium)
            .or_else(|| known_chromium_browsers().into_iter().next())
            .expect("no Chromium-family browser installed");
        eprintln!("browser: {} at {}", browser.name, browser.exe.display());

        let nonce = u64::from_le_bytes(
            crate::crypto::random_bytes(8).try_into().unwrap(),
        );
        let profile = std::env::temp_dir().join(format!("ram-sysbrowser-test-{nonce:016x}"));
        assert!(!profile.exists());
        eprintln!("profile: {}", profile.display());

        let session = open_signed_in(
            &browser,
            &profile,
            Some("test-not-a-real-cookie"),
            "about:blank",
            "RAM test",
        )
        .expect("open_signed_in");
        eprintln!("session pid: {}", session.pid);
        assert_eq!(session.profile_dir, profile);
        assert!(session.pid != 0, "pid resolved");
        assert!(is_running(&profile), "is_running after open");

        // Chromium quirk check: what targets are open after launch?
        {
            let mut cdp = Cdp::connect(&profile).unwrap();
            let targets = cdp.call("Target.getTargets", json!({})).unwrap();
            let list: Vec<String> = targets["targetInfos"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| format!("{}:{}", t["type"].as_str().unwrap_or("?"), t["url"].as_str().unwrap_or("?")))
                .collect();
            eprintln!("targets after open: {list:?}");
            let pages = list.iter().filter(|t| t.starts_with("page:")).count();
            assert_eq!(pages, 1, "exactly one page tab should remain");
            cdp.close();
        }

        let got = read_cookie(&profile).expect("read_cookie");
        assert_eq!(got.as_deref(), Some("test-not-a-real-cookie"));
        eprintln!("cookie read back OK");

        // A second open on the live profile must reuse the instance.
        let again = open_signed_in(&browser, &profile, None, "about:blank", "RAM test")
            .expect("second open_signed_in");
        assert_eq!(again.pid, session.pid, "same browser process reused");
        eprintln!("second open reused pid {}", again.pid);

        close(&profile).expect("close");
        assert!(wait_exit(session.pid, Duration::from_secs(20)), "browser exited");
        eprintln!("browser exited");
        assert!(!is_running(&profile), "not running after close");

        clear_cookies(&profile).expect("clear_cookies");
        assert!(!profile.join("Default").join("Network").join("Cookies").exists());
        wipe_profile(&profile).expect("wipe_profile");
        assert!(!profile.exists());
        eprintln!("profile wiped");
    }
}


// ---------------------------------------------------------------------------
// Pages: open one, and fill Roblox's sign-in form
// ---------------------------------------------------------------------------

/// Opens `url` in a new tab of the running browser on `profile_dir` and
/// closes every other tab, so a queue of sign-ins never piles tabs up.
pub fn open_page(profile_dir: &Path, url: &str) -> Result<(), String> {
    let mut cdp = Cdp::connect(profile_dir).map_err(|_| "The browser window is not open".to_string())?;
    let created = cdp.call("Target.createTarget", json!({ "url": url, "newWindow": false }))?;
    let keep = created.get("targetId").and_then(Value::as_str).unwrap_or("").to_string();
    if let Ok(list) = cdp.call("Target.getTargets", json!({})) {
        let others: Vec<String> = list
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
            .filter_map(|t| t.get("targetId").and_then(Value::as_str).map(str::to_string))
            .filter(|id| *id != keep)
            .collect();
        for id in others {
            let _ = cdp.call("Target.closeTarget", json!({ "targetId": id }));
        }
    }
    cdp.close();
    Ok(())
}

/// What `fill_login` managed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFill {
    /// The form was filled and its Sign in button pressed.
    Submitted,
    /// The form was filled; the button was not found, so the user presses it.
    Filled,
    /// No sign-in form appeared on the page.
    NoForm,
}

/// The script that fills Roblox's sign-in form. The values are embedded as
/// JSON string literals, so no character in a password can break out.
fn login_script(username: &str, password: &str, submit: bool) -> String {
    let u = serde_json::to_string(username).unwrap_or_else(|_| "\"\"".into());
    let p = serde_json::to_string(password).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(async (u, p, submit) => {{
  const wait = (sel, ms) => new Promise((res) => {{
    const t0 = Date.now();
    const tick = () => {{ const el = document.querySelector(sel); if (el) return res(el); if (Date.now() - t0 > ms) return res(null); setTimeout(tick, 150); }};
    tick();
  }});
  const user = await wait('#login-username', 7000);
  const pass = user ? await wait('#login-password', 1500) : null;
  if (!user || !pass) return 'no-form';
  const set = (el, v) => {{
    const d = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
    el.focus(); d.set.call(el, v);
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }};
  set(user, u); set(pass, p);
  if (!submit) return 'filled';
  await new Promise((r) => setTimeout(r, 250));
  const form = user.closest('form');
  const btn = document.querySelector('#login-button') || (form && form.querySelector('button[type="submit"]'));
  if (btn) {{ btn.click(); return 'submitted'; }}
  return 'filled';
}})({u}, {p}, {submit})"#
    )
}

/// Fills (and optionally submits) Roblox's sign-in form in the tab showing
/// it. The password travels only over the browser's loopback DevTools
/// socket; it is never part of an error string.
pub fn fill_login(profile_dir: &Path, username: &str, password: &str, submit: bool) -> Result<LoginFill, String> {
    let mut cdp = Cdp::connect(profile_dir).map_err(|_| "The browser window is not open".to_string())?;
    let list = cdp.call("Target.getTargets", json!({}))?;
    let pages: Vec<(String, String)> = list
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|t| Some((t.get("targetId")?.as_str()?.to_string(), t.get("url").and_then(Value::as_str).unwrap_or("").to_string())))
        .collect();
    let target = pages
        .iter()
        .find(|(_, url)| url.contains("roblox.com"))
        .or_else(|| pages.first())
        .map(|(id, _)| id.clone())
        .ok_or("The sign-in page is not open")?;
    let attached = cdp.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }))?;
    let session = attached.get("sessionId").and_then(Value::as_str).ok_or("could not attach to the sign-in page")?.to_string();
    let result = cdp.call_in(
        Some(&session),
        "Runtime.evaluate",
        json!({ "expression": login_script(username, password, submit), "awaitPromise": true, "returnByValue": true }),
    );
    let _ = cdp.call("Target.detachFromTarget", json!({ "sessionId": session }));
    cdp.close();
    // Never surface the protocol error text: it can echo the expression.
    let value = result.map_err(|_| "the sign-in form could not be filled".to_string())?;
    Ok(match value.get("result").and_then(|r| r.get("value")).and_then(Value::as_str) {
        Some("submitted") => LoginFill::Submitted,
        Some("filled") => LoginFill::Filled,
        _ => LoginFill::NoForm,
    })
}

#[cfg(test)]
mod login_tests {
    use super::*;

    #[test]
    fn login_script_embeds_values_as_json_literals() {
        let s = login_script("bob_1", "p\"a'ss\\word\n</script>", true);
        assert!(s.contains("(\"bob_1\", \"p\\\"a'ss\\\\word\\n</script>\", true)"), "{s}");
        assert!(s.contains("#login-username") && s.contains("button[type=\"submit\"]"));
        assert!(login_script("a", "b", false).ends_with("(\"a\", \"b\", false)"));
    }

    /// Opens a real Chromium browser on a scratch profile showing a local
    /// stand-in for Roblox's sign-in form (same field ids), fills and submits
    /// it, and reads back what the page received. Never touches roblox.com.
    #[test]
    #[ignore = "opens a browser window"]
    fn fill_login_round_trip() {
        let browser = match default_browser() {
            Some(b) if b.chromium => b,
            _ => known_chromium_browsers().into_iter().next().expect("a Chromium browser"),
        };
        let dir = std::env::temp_dir().join(format!("ram-login-test-{}", std::process::id()));
        let page = "data:text/html,<title>form</title><form name=loginForm onsubmit=\"document.title='ok-'+document.getElementById('login-username').value+'-'+document.getElementById('login-password').value.length;return false\"><input id=login-username><input id=login-password type=password><button type=submit>Sign in</button></form>";
        let session = open_signed_in(&browser, &dir, None, page, "test").expect("browser opens");
        // Collect everything first, then always close the browser, then
        // assert: a failed assertion must not leave a browser running.
        std::thread::sleep(Duration::from_millis(800));
        let got = fill_login(&dir, "bob_1", "p\"a'ss word", true);
        std::thread::sleep(Duration::from_millis(400));
        let titles = |dir: &Path| -> Vec<String> {
            let Ok(mut cdp) = Cdp::connect(dir) else { return Vec::new() };
            let list = cdp.call("Target.getTargets", json!({})).unwrap_or(Value::Null);
            cdp.close();
            list["targetInfos"].as_array().map(|a| a.iter().filter(|t| t["type"] == "page").filter_map(|t| t["title"].as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        let after_fill = titles(&dir);
        let opened = open_page(&dir, "about:blank");
        let after_open = titles(&dir);
        let closed = close(&dir);
        let exited = wait_exit(session.pid, Duration::from_secs(20));
        let _ = wipe_profile(&dir);

        assert_eq!(got, Ok(LoginFill::Submitted));
        assert!(after_fill.iter().any(|t| t == "ok-bob_1-11"), "{after_fill:?}");
        // open_page replaces the tab instead of piling tabs up.
        assert!(opened.is_ok());
        assert_eq!(after_open.len(), 1, "{after_open:?}");
        assert!(closed.is_ok() && exited);
    }
}
