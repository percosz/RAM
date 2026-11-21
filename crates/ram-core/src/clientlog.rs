//! Roblox client log reading for the crash history.
//!
//! Roblox writes one log per client to
//! `%LOCALAPPDATA%\Roblox\logs\<version>_<yyyymmddThhmmssZ>_Player_<5hex>_last.log`
//! (the matching `..._CrashHandler_last.log` is the crash handler's own log
//! and is ignored). Each line is
//!
//! ```text
//! <ISO8601>,<secs>,<thread hex>,<level>[,<Severity>] [<FLog channel>] message
//! ```
//!
//! [`parse_line`] folds one line into [`LogFacts`]; [`classify`] turns the
//! facts plus what RAM knows about the process into an [`EndKind`]. The only
//! I/O lives in [`Tail`] and [`read_head`]. Log contents are untrusted: lines
//! on the `Output`, `Warning` and `Error` channels are ignored (experience
//! scripts print anything there) except the join line, and nothing from a log
//! is stored beyond the extracted numbers and a filtered reason string.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Start of the message the client prints when it connects to a game server.
pub const JOINING: &str = "! Joining game '";
/// Lowercase words that mark a line as carrying a credential or ticket.
pub const SENSITIVE: [&str; 7] = [
    "gameinfo",
    "ticket",
    "placelauncher",
    "roblosecurity",
    "cookie",
    "accesscode",
    "linkcode",
];
/// Message printed when the experience teleports the player.
pub const TELEPORT: &str = "UgcExperienceController: doTeleport";
/// Messages that mark the end of a server connection.
pub const DISCONNECTS: [&str; 6] = [
    "Disconnection Notification",
    "Lost connection with reason",
    "Time to disconnect replication data",
    "Sending disconnect with reason",
    "Disconnected from server for reason",
    "Connection closed by server",
];
/// Messages that mark a refused sign-in before the client joined.
pub const AUTH_FAILED: [&str; 2] = ["encountered during authentication", "Error Code: 403"];
/// Messages proving the client has read its ClientAppSettings/GlobalBasicSettings.
pub const READ_MARKS: [&str; 3] = [
    "SceneManager: resizing main targets",
    "[FLog::Network] Replicator created",
    "LoadClientSettingsFromLocal",
];

/// The weakest disconnect marker: it also appears on teleports and shutdowns.
const WEAK_DISCONNECT: &str = "Time to disconnect replication data";
/// Messages meaning another client signed in with the same account.
const REPLACED_MARKS: [&str; 2] = ["Same account launched", "DuplicateLogin"];
/// Prefixes of the browser tracker id in the log.
const TRACKER_PREFIXES: [&str; 3] = ["BrowserTrackerId=", "websiteBTId is ", "BTID is overriden to "];
/// Channels that experience scripts can write to.
const NOISE_CHANNELS: [&str; 3] = ["Output", "Warning", "Error"];
/// Largest number of bytes one [`Tail::read_new`] call consumes.
const READ_CAP: u64 = 16 * 1024 * 1024;

/// What a client log has said so far.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogFacts {
    /// From `BrowserTrackerId=<n>`, `websiteBTId is <n>` or `BTID is overriden to <n>`.
    pub browser_tracker_id: Option<u64>,
    /// From `[FLog::GameJoinLoadTime] Report game_join_loadtime: ... userid:<n>`.
    pub user_id: Option<u64>,
    /// From the [`JOINING`] line: `... place <n> at`.
    pub place_id: Option<u64>,
    /// The 36-character guid in the [`JOINING`] line.
    pub job_id: Option<String>,
    /// Seconds-since-start column of the latest [`JOINING`] line.
    pub joined_at: Option<f64>,
    /// `[FLog::Network] Replicator created` or `[FLog::Network] serverId:` after a join.
    pub in_game: bool,
    /// [`TELEPORT`] seen while in game.
    pub teleporting: bool,
    /// Any [`DISCONNECTS`] marker since the latest join.
    pub disconnected: bool,
    /// The client ended the connection itself (see [`client_initiated`]).
    pub left: bool,
    /// `left` because another client took over the account (see [`EndKind::Replaced`]).
    pub replaced: bool,
    /// First disconnect line naming a reason (see [`reason_of`]); the
    /// "Time to disconnect" line only counts when nothing better came.
    pub reason: Option<String>,
    /// [`AUTH_FAILED`] seen while not in game.
    pub auth_failed: bool,
    /// `challengedByGcs` seen.
    pub challenged: bool,
    /// `[FLog::SingleSurfaceApp] shutDown` or `Received close main window request`.
    pub shutdown: bool,
    /// Any [`READ_MARKS`] seen.
    pub read_settings: bool,
    /// Seconds column of the last parsed line.
    pub last_secs: f64,
    /// Number of [`JOINING`] lines; teleports increment it.
    pub joins: u32,
    /// `reason` came from the weak "Time to disconnect" marker.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reason_is_weak: bool,
}

/// Why a client ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndKind {
    /// The user, RAM or the client itself closed the window.
    Closed,
    /// The server dropped the connection.
    Disconnected,
    /// The process vanished without saying goodbye.
    Crashed,
    /// Informational only, never a record: the client moved to another place.
    Teleported,
    /// Another client signed in with the same account.
    Replaced,
    /// Roblox refused the sign-in before the client joined.
    SignedOut,
}

/// Human label for an [`EndKind`].
pub fn kind_label(k: EndKind) -> &'static str {
    match k {
        EndKind::Closed => "Closed",
        EndKind::Disconnected => "Disconnected by Roblox",
        EndKind::Crashed => "Crashed",
        EndKind::Teleported => "Teleported",
        EndKind::Replaced => "Replaced by another client",
        EndKind::SignedOut => "Sign-in refused",
    }
}

/// Decides how a client ended; `None` while the process is still running.
///
/// Order: a closing marker (RAM closed it, the client shut down, or the
/// client left on its own) wins, as `Replaced` when the client left because
/// the account signed in elsewhere; then a refused sign-in before joining;
/// then a crash dump; then a server-side disconnect. After that the process
/// exit code decides, when it is known: an exception code is `Crashed`, and
/// anything else is `Closed`, because a client that is ended from outside
/// (Task Manager's End task, a bootstrapper or another tool killing it)
/// stops mid-line with no closing marker and no dump, exactly like a crash,
/// and only the exit code tells the two apart. Without an exit code, a
/// client that never got in game and exited within 5 s is `Closed` (an early
/// exit such as the updater relaunching a newer binary or a launch RAM
/// cancelled), after 5 s it is `Crashed`, and a client that was in game and
/// silently vanished is `Crashed`.
pub fn classify(
    f: &LogFacts,
    process_exited: bool,
    crash_dump_seen: bool,
    closed_by_ram: bool,
    exit_code: Option<u32>,
) -> Option<EndKind> {
    if !process_exited {
        return None;
    }
    if closed_by_ram || f.shutdown || f.left {
        return Some(if f.replaced { EndKind::Replaced } else { EndKind::Closed });
    }
    if f.auth_failed && !f.in_game {
        return Some(EndKind::SignedOut);
    }
    if crash_dump_seen {
        return Some(EndKind::Crashed);
    }
    if f.disconnected {
        return Some(EndKind::Disconnected);
    }
    if let Some(code) = exit_code {
        return Some(if exit_is_crash(code) { EndKind::Crashed } else { EndKind::Closed });
    }
    if !f.in_game {
        return Some(if f.last_secs > 5.0 { EndKind::Crashed } else { EndKind::Closed });
    }
    Some(EndKind::Crashed)
}

/// Whether a Windows process exit code is an unhandled exception or a hang.
///
/// A crash ends the process with an NTSTATUS of warning or error severity
/// (`0xC0000005` access violation, `0xC0000409` stack buffer overrun,
/// `0x80000003` breakpoint, `0xE06D7363` C++ exception, `0xCFFFFFFF` hang
/// ended by Windows). A process that is ended by someone carries the code
/// that caller chose: `1` from Task Manager and `taskkill`, `0xFFFFFFFF` from
/// .NET's `Process.Kill` (bootstrappers, PowerShell), and `0xC000013A` when
/// a console is closed. A normal exit is `0`.
pub fn exit_is_crash(code: u32) -> bool {
    const CONTROL_C_EXIT: u32 = 0xC000_013A;
    (0x8000_0000..0xF000_0000).contains(&code) && code != CONTROL_C_EXIT
}

/// Folds one complete line (without its newline) into `f`.
///
/// Lines on the `Output`, `Warning` and `Error` channels are ignored except
/// the [`JOINING`] line and a [`READ_MARKS`] entry at the very start of the
/// message (`LoadClientSettingsFromLocal` is printed on `Output`). Lines
/// without the timestamp prefix (the licence banner, continuation lines of
/// multi-line messages) are ignored.
pub fn parse_line(f: &mut LogFacts, line: &str) {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((secs, rest)) = split_prefix(line) else {
        return;
    };
    if let Some(secs) = secs {
        f.last_secs = secs;
    }
    let (channel, message) = split_channel(rest);
    let noisy = channel.is_some_and(|c| NOISE_CHANNELS.contains(&c));

    if let Some(after) = find_after(message, JOINING) {
        f.joins += 1;
        f.joined_at = secs;
        f.job_id = after.get(..36).filter(|g| is_guid(g)).map(str::to_owned);
        if let Some(place) = find_after(after, " place ") {
            f.place_id = leading_number(place).or(f.place_id);
        }
        // A fresh connection: forget the previous one's end.
        f.in_game = false;
        f.disconnected = false;
        f.left = false;
        f.replaced = false;
        f.reason = None;
        f.reason_is_weak = false;
        return;
    }
    if READ_MARKS.iter().any(|m| message.starts_with(m)) {
        f.read_settings = true;
    }
    if noisy {
        return;
    }
    if READ_MARKS.iter().any(|m| line.contains(m)) {
        f.read_settings = true;
    }

    for prefix in TRACKER_PREFIXES {
        if let Some(n) = find_after(message, prefix).and_then(leading_number) {
            f.browser_tracker_id = Some(n);
        }
    }
    if message.contains("Report game_join_loadtime:") {
        if let Some(n) = find_after(message, "userid:").and_then(leading_number) {
            f.user_id = Some(n);
        }
        if f.place_id.is_none() {
            f.place_id = find_after(message, "placeid:").and_then(leading_number);
        }
    }
    if f.joins > 0
        && (line.contains("[FLog::Network] Replicator created")
            || line.contains("[FLog::Network] serverId:"))
    {
        f.in_game = true;
    }
    if f.in_game && message.contains(TELEPORT) {
        f.teleporting = true;
    }

    let mut on_disconnect = false;
    for marker in DISCONNECTS {
        if !message.contains(marker) {
            continue;
        }
        on_disconnect = true;
        f.disconnected = true;
        let weak = marker == WEAK_DISCONNECT;
        if f.reason.is_none() || (f.reason_is_weak && !weak) {
            if let Some(r) = reason_of(message, marker) {
                f.reason = Some(r);
                f.reason_is_weak = weak;
            }
        }
    }
    let replaced = REPLACED_MARKS.iter().any(|m| message.contains(m))
        || (on_disconnect && standalone_number(message, "273"));
    if replaced {
        f.left = true;
        f.replaced = true;
    } else if (on_disconnect || message.contains("DisconnectClientInitiated"))
        && client_initiated(message)
    {
        f.left = true;
    }

    if !f.in_game && AUTH_FAILED.iter().any(|m| message.contains(m)) {
        f.auth_failed = true;
    }
    if message.contains("challengedByGcs") {
        f.challenged = true;
    }
    if line.contains("[FLog::SingleSurfaceApp] shutDown")
        || message.contains("Received close main window request")
    {
        f.shutdown = true;
    }
}

/// Folds every complete (newline-terminated) line of `text` into `f`.
pub fn parse_text(f: &mut LogFacts, text: &str) {
    for line in text.split_inclusive('\n') {
        if line.ends_with('\n') {
            parse_line(f, line);
        }
    }
}

/// The text from `marker` on, keeping letters, digits, spaces and `.,:;()_-`,
/// at most 160 characters, trimmed. `None` when the marker is absent or
/// nothing printable remains.
pub fn reason_of(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)?;
    let kept: String = line[start..]
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || ".,:;()_-".contains(*c))
        .take(160)
        .collect();
    let kept = kept.trim();
    (!kept.is_empty()).then(|| kept.to_owned())
}

/// Whether a disconnect line says the client ended the connection itself:
/// `DisconnectClientInitiated`, `Same account launched`, `DuplicateLogin`, or
/// the standalone reason numbers 285 (client left) and 273 (duplicate login).
pub fn client_initiated(line: &str) -> bool {
    line.contains("DisconnectClientInitiated")
        || REPLACED_MARKS.iter().any(|m| line.contains(m))
        || standalone_number(line, "285")
        || standalone_number(line, "273")
}

/// Whether `name` is a player log (not the crash handler's).
pub fn is_player_log_name(name: &str) -> bool {
    name.contains("_Player_") && name.contains("_last.log") && !name.contains("_CrashHandler_")
}

/// The `yyyymmddThhmmssZ` stamp in a log name as unix seconds.
pub fn log_start_time(name: &str) -> Option<i64> {
    let b = name.as_bytes();
    (0..b.len().saturating_sub(15)).find_map(|i| {
        let s = &b[i..i + 16];
        let ok = s[..8].iter().all(u8::is_ascii_digit)
            && s[8] == b'T'
            && s[9..15].iter().all(u8::is_ascii_digit)
            && s[15] == b'Z';
        if !ok {
            return None;
        }
        let num = |r: std::ops::Range<usize>| std::str::from_utf8(&s[r]).ok()?.parse::<i64>().ok();
        utc_to_unix(num(0..4)?, num(4..6)?, num(6..8)?, num(9..11)?, num(11..13)?, num(13..15)?)
    })
}

/// Whether the head of a log names this browser tracker id (the id must not
/// continue with another digit).
pub fn tracker_in_head(head: &str, browser_tracker_id: &str) -> bool {
    if browser_tracker_id.is_empty() {
        return false;
    }
    TRACKER_PREFIXES.iter().any(|prefix| {
        let needle = format!("{prefix}{browser_tracker_id}");
        let mut from = 0;
        while let Some(pos) = head[from..].find(&needle) {
            let end = from + pos + needle.len();
            if !head[end..].starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
            from = end;
        }
        false
    })
}

/// Whether a line may carry a credential, ticket or URL.
pub fn is_sensitive(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("http") || SENSITIVE.iter().any(|w| lower.contains(w))
}

/// Incremental reader of one log file.
pub struct Tail {
    pub path: PathBuf,
    /// Bytes already consumed.
    pub offset: u64,
}

impl Tail {
    /// A tail starting at the beginning of `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Tail { path: path.into(), offset: 0 }
    }

    /// Parses the complete lines appended since the last call and returns how
    /// many bytes the offset advanced. A file shorter than the offset was
    /// replaced and is read from the start. At most 16 MiB are read per call:
    /// when more is pending the reader skips to the tail and drops the first
    /// partial line.
    pub fn read_new(&mut self, facts: &mut LogFacts) -> io::Result<usize> {
        let mut file = open_shared(&self.path)?;
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
        }
        let before = self.offset;
        let mut start = self.offset;
        let skipping = len - start > READ_CAP;
        if skipping {
            start = len - READ_CAP;
        }
        file.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::with_capacity((len - start).min(READ_CAP) as usize);
        file.take(READ_CAP).read_to_end(&mut buf)?;

        let mut head = 0;
        if skipping {
            head = match buf.iter().position(|&b| b == b'\n') {
                Some(nl) => nl + 1,
                None => buf.len(),
            };
        }
        let end = match buf[head..].iter().rposition(|&b| b == b'\n') {
            Some(nl) => head + nl + 1,
            None => head,
        };
        let text = String::from_utf8_lossy(&buf[head..end]);
        parse_text(facts, &text);
        self.offset = start + end as u64;
        Ok((self.offset - before) as usize)
    }
}

/// The first `max` bytes of `path`, decoded lossily.
pub fn read_head(path: &Path, max: usize) -> io::Result<String> {
    let file = open_shared(path)?;
    let mut buf = Vec::with_capacity(max.min(64 * 1024));
    file.take(max as u64).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Opens a file for reading while Roblox keeps it open for writing.
fn open_shared(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        options.share_mode(0x7);
    }
    options.open(path)
}

/// One RFC 4180 record with a CRLF end. Cells containing `,`, `"`, CR or LF
/// are quoted with inner quotes doubled; a cell starting with `=`, `+`, `-`
/// or `@` gets a leading `'` so spreadsheets do not run it as a formula.
pub fn csv_line<S: AsRef<str>>(cells: &[S]) -> String {
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let cell = cell.as_ref();
        let defused = cell.starts_with(['=', '+', '-', '@']);
        let quote = cell.contains([',', '"', '\r', '\n']);
        if quote {
            out.push('"');
        }
        if defused {
            out.push('\'');
        }
        if quote {
            out.push_str(&cell.replace('"', "\"\""));
        } else {
            out.push_str(cell);
        }
        if quote {
            out.push('"');
        }
    }
    out.push_str("\r\n");
    out
}

/// A CSV document with a UTF-8 BOM, a header record and one record per row.
pub fn csv_document(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::from("\u{FEFF}");
    out.push_str(&csv_line(head));
    for row in rows {
        out.push_str(&csv_line(row));
    }
    out
}

// ---- helpers ---------------------------------------------------------------

/// Splits the `<ISO8601>,<secs>,<thread>,<level>[,<Severity>] ` prefix off a
/// line, returning the seconds column (if it parses) and the remainder.
/// `None` for lines without the timestamp prefix.
fn split_prefix(line: &str) -> Option<(Option<f64>, &str)> {
    let (stamp, rest) = line.split_once(',')?;
    if stamp.len() < 20 || !stamp.starts_with(|c: char| c.is_ascii_digit()) || !stamp.ends_with('Z') {
        return None;
    }
    let (secs, rest) = rest.split_once(',')?;
    let secs = secs.parse::<f64>().ok().filter(|s| s.is_finite());
    // thread and level[,severity] run up to the first space.
    let (_, rest) = rest.split_once(' ')?;
    Some((secs, rest))
}

/// Splits `[FLog::Channel] message` into the channel name and the message.
fn split_channel(rest: &str) -> (Option<&str>, &str) {
    let Some(tag) = rest.strip_prefix('[') else {
        return (None, rest);
    };
    let Some(close) = tag.find(']') else {
        return (None, rest);
    };
    let channel = tag[..close].rsplit("::").next().unwrap_or("");
    let message = tag[close + 1..].trim_start_matches(' ');
    (Some(channel), message)
}

/// The text after the first occurrence of `needle`.
fn find_after<'a>(hay: &'a str, needle: &str) -> Option<&'a str> {
    hay.find(needle).map(|i| &hay[i + needle.len()..])
}

/// The decimal number at the start of `s`.
fn leading_number(s: &str) -> Option<u64> {
    let digits: &str = s.split(|c: char| !c.is_ascii_digit()).next()?;
    (!digits.is_empty() && digits.len() <= 19)
        .then(|| digits.parse().ok())
        .flatten()
}

/// Whether `s` is a `8-4-4-4-12` hex guid.
fn is_guid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// Whether `number` occurs in `line` without a digit on either side.
fn standalone_number(line: &str, number: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = line[from..].find(number) {
        let at = from + pos;
        let end = at + number.len();
        let before = line[..at].chars().next_back().is_some_and(|c| c.is_ascii_digit());
        let after = line[end..].starts_with(|c: char| c.is_ascii_digit());
        if !before && !after {
            return true;
        }
        from = end;
    }
    false
}

/// Unix seconds for a proleptic Gregorian UTC date-time.
fn utc_to_unix(y: i64, m: i64, d: i64, hh: i64, mm: i64, ss: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a log line in the client's format.
    fn l(secs: f64, channel: &str, msg: &str) -> String {
        format!("2026-11-21T02:56:21.378Z,{secs:.6},0724,6 [FLog::{channel}] {msg}\n")
    }

    fn join_line(secs: f64) -> String {
        l(
            secs,
            "Output",
            "! Joining game '51e6a897-1176-40ee-ab5b-1effb0ba3dda' place 3016661674 at 10.186.4.211",
        )
    }

    fn in_game_lines(secs: f64) -> String {
        l(secs, "Network", "serverId: 128.116.63.33|53330")
            + &l(secs, "Network", "Replicator created for player 128.116.63.33|53330")
            + &l(secs, "Graphics", "SceneManager: resizing main targets to 2560x1440")
    }

    fn facts(text: &str) -> LogFacts {
        let mut f = LogFacts::default();
        parse_text(&mut f, text);
        f
    }

    #[test]
    fn join_then_server_drop_is_disconnected() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(90.0, "Network", "Time to disconnect replication data: 0.010700");
        let f = facts(&text);
        assert!(f.in_game && f.disconnected && !f.left);
        assert_eq!(f.joins, 1);
        assert_eq!(f.place_id, Some(3016661674));
        assert_eq!(f.job_id.as_deref(), Some("51e6a897-1176-40ee-ab5b-1effb0ba3dda"));
        assert_eq!(f.joined_at, Some(1.4));
        assert!(f.read_settings);
        assert_eq!(f.last_secs, 90.0);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Disconnected));
        assert_eq!(classify(&f, false, false, false, None), None);
    }

    #[test]
    fn teleport_counts_joins_and_is_not_an_end() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(9.1, "UgcExperienceController", "UgcExperienceController: doTeleport: url ")
            + &l(9.14, "Network", "Time to disconnect replication data: 0.010700")
            + &l(
                9.57,
                "Output",
                "! Joining game '6bc17512-7df1-4af8-a378-b456345be0db' place 5208655184 at 10.221.37.7",
            )
            + &in_game_lines(9.7);
        let f = facts(&text);
        assert_eq!(f.joins, 2);
        assert!(f.teleporting);
        assert!(f.in_game);
        assert!(!f.disconnected);
        assert_eq!(f.place_id, Some(5208655184));
        assert_eq!(classify(&f, false, false, false, None), None);
        // Vanishing after the teleport is a crash, not the teleport's disconnect.
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Crashed));
    }

    #[test]
    fn reason_285_is_client_closed() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(142.0, "Network", "Sending disconnect with reason: 285")
            + &l(142.0, "Network", "Disconnection Notification. Reason: 285");
        let f = facts(&text);
        assert!(f.left && f.disconnected && !f.replaced);
        assert_eq!(f.reason.as_deref(), Some("Sending disconnect with reason: 285"));
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Closed));
    }

    #[test]
    fn same_account_is_replaced() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(50.0, "Network", "Lost connection with reason : Same account launched elsewhere");
        let f = facts(&text);
        assert!(f.left && f.replaced);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Replaced));

        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(50.0, "Network", "Disconnection Notification. Reason: 273");
        assert_eq!(classify(&facts(&text), true, false, false, None), Some(EndKind::Replaced));
    }

    #[test]
    fn shutdown_is_closed() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(223.9, "SingleSurfaceApp", "shutDown: (stage:UGCGame).")
            + &l(224.3, "Network", "Time to disconnect replication data: 1.845500");
        let f = facts(&text);
        assert!(f.shutdown);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Closed));
        assert_eq!(classify(&f, true, true, false, None), Some(EndKind::Closed));
    }

    #[test]
    fn never_joined_depends_on_lifetime() {
        let early = facts(&l(2.0, "RobloxStarter", "Starting module: Network"));
        assert_eq!(classify(&early, true, false, false, None), Some(EndKind::Closed));
        let late = facts(&l(40.0, "RobloxStarter", "Starting module: Network"));
        assert_eq!(classify(&late, true, false, false, None), Some(EndKind::Crashed));
        assert_eq!(classify(&late, true, false, true, None), Some(EndKind::Closed));
    }

    #[test]
    fn exit_code_tells_a_kill_from_a_crash() {
        // In game, the log simply stops: no closing marker, no dump.
        let f = facts(&(join_line(1.4) + &in_game_lines(2.4) + &l(5339.1, "CreatorOutput", "2")));
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Crashed));
        // Ended from outside: Task Manager (1), .NET Process.Kill (-1), exit(0).
        for code in [0, 1, 0xFFFF_FFFF, 0xC000_013A, 0x4001_0004] {
            assert!(!exit_is_crash(code), "{code:#x}");
            assert_eq!(classify(&f, true, false, false, Some(code)), Some(EndKind::Closed), "{code:#x}");
        }
        // Unhandled exceptions and a hang ended by Windows.
        for code in [0xC000_0005, 0xC000_0409, 0x8000_0003, 0xE06D_7363, 0xCFFF_FFFF, 0xC000_001D] {
            assert!(exit_is_crash(code), "{code:#x}");
            assert_eq!(classify(&f, true, false, false, Some(code)), Some(EndKind::Crashed), "{code:#x}");
        }
        // A crash dump is a crash whatever the code says; the log's own
        // markers still win over the code.
        assert_eq!(classify(&f, true, true, false, Some(1)), Some(EndKind::Crashed));
        let dropped = facts(&(join_line(1.4) + &in_game_lines(2.4) + &l(90.0, "Network", "Connection closed by server for reason: 288")));
        assert_eq!(classify(&dropped, true, false, false, Some(1)), Some(EndKind::Disconnected));
        let shut = facts(&(join_line(1.4) + &in_game_lines(2.4) + &l(90.0, "SingleSurfaceApp", "shutDown: (stage:UGCGame).")));
        assert_eq!(classify(&shut, true, false, false, Some(0xC000_0005)), Some(EndKind::Closed));
        // Never joined, ended from outside after a long wait: closed, not crashed.
        let late = facts(&l(40.0, "RobloxStarter", "Starting module: Network"));
        assert_eq!(classify(&late, true, false, false, Some(1)), Some(EndKind::Closed));
        assert_eq!(classify(&late, true, false, false, Some(0xC000_0005)), Some(EndKind::Crashed));
        assert_eq!(classify(&f, false, false, false, Some(1)), None);
    }

    #[test]
    fn auth_failure_is_signed_out() {
        let text = l(0.5, "RobloxStarter", "Starting module: Network")
            + &l(1.2, "Auth", "An error was encountered during authentication. Error Code: 403");
        let f = facts(&text);
        assert!(f.auth_failed && !f.in_game);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::SignedOut));
        // After joining the same text is just chatter.
        let f = facts(&(join_line(1.0) + &in_game_lines(2.0) + &l(3.0, "Auth", "Error Code: 403")));
        assert!(!f.auth_failed);
    }

    #[test]
    fn reason_keeps_printable_text_and_prefers_strong_markers() {
        let line = "Connection closed by server for reason: RCC: 288 (DisconnectServerShutdown) \u{1F4A5}<x>\t\"y\"";
        assert_eq!(
            reason_of(line, "Connection closed by server").as_deref(),
            Some("Connection closed by server for reason: RCC: 288 (DisconnectServerShutdown) xy")
        );
        assert_eq!(reason_of(line, "absent"), None);
        let long = format!("Lost connection with reason {}", "x".repeat(400));
        assert_eq!(reason_of(&long, "Lost connection").unwrap().chars().count(), 160);

        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(50.0, "Network", "Time to disconnect replication data: 1.0")
            + &l(50.1, "Network", "Connection closed by server for reason: RCC: 288 (DisconnectServerShutdown)")
            + &l(50.2, "Network", "Disconnection Notification. Reason: 288");
        let f = facts(&text);
        assert_eq!(
            f.reason.as_deref(),
            Some("Connection closed by server for reason: RCC: 288 (DisconnectServerShutdown)")
        );
        assert!(!f.reason_is_weak);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Disconnected));
    }

    #[test]
    fn script_output_is_ignored() {
        let text = join_line(1.4)
            + &in_game_lines(2.4)
            + &l(5.0, "Output", "Disconnection Notification. Reason: 285 shutDown challengedByGcs")
            + &l(5.1, "Warning", "Lost connection with reason: Same account launched")
            + &l(5.2, "Error", "Received close main window request")
            + &l(5.3, "Output", "websiteBTId is 999");
        let f = facts(&text);
        assert!(!f.disconnected && !f.left && !f.shutdown && !f.challenged);
        assert_eq!(f.browser_tracker_id, None);
        assert_eq!(f.last_secs, 5.3);
        assert_eq!(classify(&f, true, false, false, None), Some(EndKind::Crashed));
        // The licence banner and continuation lines carry no prefix and are skipped.
        let f = facts("[FLog::Output] All use of Roblox services must comply.\n    \"Disconnection Notification\": 1\n");
        assert_eq!(f, LogFacts::default());
    }

    #[test]
    fn extracts_ids_from_realistic_lines() {
        let text = l(0.4, "Output", "LoadClientSettingsFromLocal: \"{")
            + "    \"FFlagX\": true\n"
            + &l(0.45, "LogWin32BTId", "LogWin32BTId, App, web launch, websiteBTId is 124468676402")
            + &l(0.46, "AppRemoteCheckStarterModule", "BTID is overriden to 124468676402.")
            + &join_line(1.4)
            + &l(
                1.4,
                "GameJoinLoadTime",
                "Report game_join_loadtime: placeid:3016661674, join_time:0.69, universeid:1087859240, referral_page:RequestFollowUser, sid:116d3297-19e1-426c-8790-fe4159d4fdec, clienttime:1795229782.15, userid:7777229606, ",
            )
            + &l(2.0, "Network", "challengedByGcs: true")
            + &l(3.0, "UserInputService", "Received close main window request");
        let f = facts(&text);
        assert!(f.read_settings);
        assert_eq!(f.browser_tracker_id, Some(124468676402));
        assert_eq!(f.user_id, Some(7777229606));
        assert_eq!(f.place_id, Some(3016661674));
        assert!(f.challenged && f.shutdown);

        let mut g = LogFacts::default();
        parse_line(&mut g, &l(0.3, "Warning", "init BrowserTrackerId=55 done"));
        assert_eq!(g.browser_tracker_id, None, "Warning channel is noise");
        parse_line(&mut g, &l(0.3, "Starter", "init BrowserTrackerId=55 done"));
        assert_eq!(g.browser_tracker_id, Some(55));
    }

    #[test]
    fn tracker_in_head_requires_a_whole_id() {
        let head = "x websiteBTId is 1244686764021\ny BTID is overriden to 124468676402.\n";
        assert!(tracker_in_head(head, "124468676402"));
        assert!(!tracker_in_head(head, "12446867640"));
        assert!(!tracker_in_head("suggestedBrowserTrackerId=1234567", "123456"));
        assert!(tracker_in_head("suggestedBrowserTrackerId=123456&x", "123456"));
        assert!(!tracker_in_head(head, ""));
    }

    #[test]
    fn sensitive_lines() {
        assert!(is_sensitive("UgcExperienceController: doTeleport: Ticket abc"));
        assert!(is_sensitive("GET HTTPS://example"));
        assert!(is_sensitive("set-COOKIE seen"));
        assert!(!is_sensitive("Replicator created: 0000"));
    }

    #[test]
    fn client_initiated_markers() {
        assert!(client_initiated("Sending disconnect with reason: 285"));
        assert!(client_initiated("reason 273"));
        assert!(client_initiated("DisconnectClientInitiated"));
        assert!(client_initiated("DuplicateLogin"));
        assert!(!client_initiated("reason: 2850"));
        assert!(!client_initiated("reason: 1285"));
        assert!(!client_initiated("reason: 288"));
    }

    #[test]
    fn log_names_and_times() {
        assert!(is_player_log_name("0.748.0.7481526_20261121T025620Z_Player_945D3_last.log"));
        assert!(!is_player_log_name("0.748.0.7481526_20261121T025621Z_Player_65B5A_CrashHandler_last.log"));
        assert!(!is_player_log_name("0.748.0.7481526_20261121T025621Z_Studio_65B5A_last.log"));
        assert!(!is_player_log_name("0.748.0.7481526_20261121T025620Z_Player_945D3.log"));

        // 2026-11-21T02:56:20Z: 20778 days since the epoch (56 years, 14 leap
        // days, 324 days into 2026) = 1795219200 s, plus 02:56:20 = 10580 s.
        assert_eq!(
            log_start_time("0.748.0.7481526_20261121T025620Z_Player_945D3_last.log"),
            Some(1795229780)
        );
        assert_eq!(log_start_time("x_19700101T000000Z_y"), Some(0));
        assert_eq!(log_start_time("x_20000301T000000Z_y"), Some(951_868_800));
        assert_eq!(log_start_time("x_20261121T025620_y"), None);
        assert_eq!(log_start_time("0.748.0.7481526_Player_945D3_last.log"), None);
        assert_eq!(log_start_time("x_20261321T025620Z_y"), None);
    }

    #[test]
    fn tail_reads_incrementally_and_after_truncation() {
        let dir = std::env::temp_dir().join(format!("ram-clientlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("0.748.0.7481526_20261121T025620Z_Player_945D3_last.log");
        let mut file = File::create(&path).unwrap();
        let first = join_line(1.4) + &in_game_lines(2.4);
        file.write_all(first.as_bytes()).unwrap();
        file.write_all(b"2026-11-21T02:56:30.000Z,10.0,0724,6 [FLog::Net").unwrap();
        file.flush().unwrap();

        let mut tail = Tail::new(&path);
        let mut f = LogFacts::default();
        let n = tail.read_new(&mut f).unwrap();
        assert_eq!(n, first.len());
        assert_eq!(tail.offset, first.len() as u64);
        assert!(f.in_game && !f.disconnected);
        assert_eq!(f.last_secs, 2.4);

        let second = "work] Time to disconnect replication data: 0.5\n";
        file.write_all(second.as_bytes()).unwrap();
        file.flush().unwrap();
        let n = tail.read_new(&mut f).unwrap();
        assert!(n > second.len());
        assert!(f.disconnected);
        assert_eq!(f.last_secs, 10.0);
        assert_eq!(tail.offset, std::fs::metadata(&path).unwrap().len());
        assert_eq!(tail.read_new(&mut f).unwrap(), 0);

        // The file was replaced by a shorter one: start over.
        drop(file);
        let fresh = l(0.5, "RobloxStarter", "Starting module: Network");
        std::fs::write(&path, &fresh).unwrap();
        let mut g = LogFacts::default();
        assert_eq!(tail.read_new(&mut g).unwrap(), fresh.len());
        assert_eq!(tail.offset, fresh.len() as u64);
        assert_eq!(g.last_secs, 0.5);

        let head = read_head(&path, 24).unwrap();
        assert_eq!(head, "2026-11-21T02:56:21.378Z");
        assert_eq!(read_head(&path, 1 << 20).unwrap(), fresh);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn csv_quoting_and_defusing() {
        assert_eq!(csv_line(&["a", "b c", "1"]), "a,b c,1\r\n");
        assert_eq!(csv_line(&["x,y", "say \"hi\"", "two\nlines"]), "\"x,y\",\"say \"\"hi\"\"\",\"two\nlines\"\r\n");
        assert_eq!(csv_line(&["=SUM(A1)", "+1", "-2", "@x", "=a,b"]), "'=SUM(A1),'+1,'-2,'@x,\"'=a,b\"\r\n");
        assert_eq!(csv_line::<&str>(&[]), "\r\n");
        let doc = csv_document(&["id", "kind"], &[vec!["1".into(), "Crashed".into()], vec!["2".into(), "=x".into()]]);
        assert_eq!(doc, "\u{FEFF}id,kind\r\n1,Crashed\r\n2,'=x\r\n");
    }

    #[test]
    fn labels_and_serde() {
        assert_eq!(kind_label(EndKind::Disconnected), "Disconnected by Roblox");
        assert_eq!(kind_label(EndKind::Replaced), "Replaced by another client");
        assert_eq!(kind_label(EndKind::SignedOut), "Sign-in refused");
        let f = facts(&(join_line(1.0) + &in_game_lines(2.0)));
        let json = serde_json::to_string(&f).unwrap();
        let back: LogFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
        assert_eq!(serde_json::to_string(&EndKind::Crashed).unwrap(), "\"Crashed\"");
    }
}
