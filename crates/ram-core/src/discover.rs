//! Finds legacy account files (`AccountData.json` and its backups) on disk,
//! so the user can pick one to import instead of browsing for it.
//!
//! The walk only looks at names and metadata. Of a file that matches it
//! reads the first bytes, enough to tell how the file is stored, and keeps
//! none of them.

use crate::import::{detect_account_file, match_account_file_name, AccountFileKind};
use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use zeroize::Zeroize;

/// Most files `find_account_files` returns.
pub const MAX_FOUND: usize = 50;

/// How much of a file is read to tell its kind.
const HEAD_BYTES: usize = 256;

/// Directories that are large and never hold a legacy install. Compared with
/// the directory's own name, ignoring ASCII case.
const SKIPPED_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "node_modules",
    ".git",
    "winsxs",
    ".cargo",
    ".rustup",
    "target",
];

/// Also skipped, by the end of its path: `AppData\Local\Temp`.
const SKIPPED_TAIL: [&str; 3] = ["appdata", "local", "temp"];

/// A legacy account file found on disk. Holds nothing of its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundFile {
    pub path: PathBuf,
    /// How the file is stored, from its first bytes. `Unrecognised` also
    /// covers a file that is empty, could not be opened, or is a cloud
    /// placeholder that is not on this PC (it is not downloaded to find out).
    pub kind: AccountFileKind,
    /// Last modification, in Unix seconds.
    pub modified: Option<i64>,
    /// In bytes.
    pub size: u64,
    /// One of the legacy app's copies (`.backup`, `.bak`, `.old`) rather
    /// than `AccountData.json` itself.
    pub is_backup: bool,
}

/// Where to look first: the places a downloaded tool usually lives, then the
/// whole home directory (which covers them again; a directory is only walked
/// once).
pub fn default_roots(home: &Path) -> Vec<PathBuf> {
    vec![home.join("Desktop"), home.join("Downloads"), home.join("Documents"), home.to_path_buf()]
}

/// Walks the roots breadth-first, in order, for files named
/// `AccountData.json`, `.backup`, `.bak` or `.old` after it (ignoring case),
/// and returns up to `MAX_FOUND` of them, most recently modified first.
///
/// A root is depth 0, so `max_depth` 0 looks at the files directly in the
/// roots and `max_depth` 1 also in their subdirectories. The walk does not
/// follow symbolic links or junctions, does not enter directories that start
/// with a dot or are in the skip list (system, package and build
/// directories; the roots themselves are always entered), and stops when
/// `budget` is used up, returning what it has. Directories that cannot be
/// read are passed over.
pub fn find_account_files(roots: &[PathBuf], max_depth: usize, budget: Duration) -> Vec<FoundFile> {
    let started = Instant::now();
    let spent = || started.elapsed() >= budget;

    // Each with whether its contents may be read.
    let mut found: Vec<(FoundFile, bool)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for root in roots {
        if seen.insert(dir_key(root)) {
            queue.push_back((root.clone(), 0));
        }
    }

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if spent() {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if spent() {
                break 'walk;
            }
            // The entry's own type: a link or junction is neither a file nor a directory here.
            let Ok(file_type) = entry.file_type() else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if file_type.is_dir() {
                let path = entry.path();
                if depth < max_depth && !is_skipped_dir(name, &path) && seen.insert(dir_key(&path)) {
                    queue.push_back((path, depth + 1));
                }
            } else if file_type.is_file() {
                let Some(is_backup) = match_account_file_name(name) else { continue };
                let (modified, size, readable) = match entry.metadata() {
                    Ok(meta) => (meta.modified().ok().map(unix_seconds), meta.len(), is_local(&meta)),
                    Err(_) => (None, 0, true),
                };
                // The kind is filled in below, for the files that are returned.
                let kind = AccountFileKind::Unrecognised;
                found.push((FoundFile { path: entry.path(), kind, modified, size, is_backup }, readable));
            }
        }
    }

    // Newest first; files without a time last; then the file before its backups.
    found.sort_by(|(a, _), (b, _)| {
        b.modified.cmp(&a.modified).then(a.is_backup.cmp(&b.is_backup)).then_with(|| a.path.cmp(&b.path))
    });
    found.truncate(MAX_FOUND);
    found
        .into_iter()
        .map(|(mut file, readable)| {
            if readable {
                file.kind = head_kind(&file.path);
            }
            file
        })
        .collect()
}

fn is_skipped_dir(name: &str, path: &Path) -> bool {
    if name.starts_with('.') || SKIPPED_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
        return true;
    }
    let mut tail = path.components().rev().map(|c| c.as_os_str().to_str().unwrap_or(""));
    SKIPPED_TAIL.iter().rev().all(|part| tail.next().is_some_and(|c| c.eq_ignore_ascii_case(part)))
}

/// Identifies a directory across the ways it can be spelled, so that it is
/// walked once when roots overlap.
fn dir_key(path: &Path) -> String {
    let key: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let key = key.join("/");
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

/// Seconds since 1970, negative before it.
fn unix_seconds(time: SystemTime) -> i64 {
    let whole = |d: Duration| i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => whole(after),
        Err(before) => -whole(before.duration()),
    }
}

/// False for a cloud placeholder whose contents would have to be downloaded
/// to be read; such a file is listed without being opened.
#[cfg(windows)]
fn is_local(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    meta.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) == 0
}

#[cfg(not(windows))]
fn is_local(_: &std::fs::Metadata) -> bool {
    true
}

/// The storage variant, from the first `HEAD_BYTES` bytes. The start of a
/// plaintext file can hold part of a cookie, so the buffer is wiped.
fn head_kind(path: &Path) -> AccountFileKind {
    let mut head = [0u8; HEAD_BYTES];
    let mut filled = 0;
    if let Ok(mut file) = std::fs::File::open(path) {
        while filled < HEAD_BYTES {
            match file.read(&mut head[filled..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => filled += n,
            }
        }
    }
    let kind = detect_account_file(&head[..filled]);
    head.zeroize();
    kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::PASSWORD_HEADER;
    use std::fs;

    const DPAPI_MAGIC: [u8; 20] = [
        0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF, 0x01, 0x15, 0xD1, 0x11, 0x8C, 0x7A, 0x00, 0xC0, 0x4F, 0xC2,
        0x97, 0xEB,
    ];
    const LONG: Duration = Duration::from_secs(30);

    struct Tree(PathBuf);
    impl Tree {
        fn new(tag: &str) -> Tree {
            let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let dir = std::env::temp_dir().join(format!("RAM-discover-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }

        /// Writes a file, creating its directories, and sets its age in days.
        fn file(&self, relative: &str, bytes: &[u8], days_old: u64) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            let when = SystemTime::now() - Duration::from_secs(days_old * 86_400);
            fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
            path
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn password_file() -> Vec<u8> {
        let mut bytes = PASSWORD_HEADER.to_vec();
        bytes.extend_from_slice(&[0x5A; 500]);
        bytes
    }

    fn dpapi_file() -> Vec<u8> {
        let mut bytes = DPAPI_MAGIC.to_vec();
        bytes.extend_from_slice(&[0xAB; 300]);
        bytes
    }

    #[test]
    fn finds_each_variant_newest_first() {
        let tree = Tree::new("variants");
        let plain = tree.file("Desktop/RAM/AccountData.json", br#"[{"Username":"a","SecurityToken":"SECRETZZ"}]"#, 3);
        let dpapi = tree.file("Downloads/old ram/ACCOUNTDATA.JSON", &dpapi_file(), 10);
        let locked = tree.file("Desktop/RAM/AccountData.json.backup", &password_file(), 1);
        let empty = tree.file("Documents/AccountData.json.bak", b"", 20);
        let old = tree.file("Documents/accountdata.json.OLD", b"not an account list", 30);
        tree.file("Desktop/RAM/RecentGames.json", b"[]", 0);
        tree.file("Desktop/RAM/AccountData.json.tmp", b"[]", 0);
        tree.file("Desktop/RAM/AccountControlData.json", b"[]", 0);
        fs::create_dir_all(tree.0.join("Documents/AccountData.json.backup")).unwrap();

        let found = find_account_files(std::slice::from_ref(&tree.0), 4, LONG);
        let got: Vec<_> = found.iter().map(|f| (f.path.clone(), f.kind.clone(), f.is_backup, f.size)).collect();
        assert_eq!(
            got,
            vec![
                (locked, AccountFileKind::PasswordProtected, true, 564),
                (plain, AccountFileKind::Plaintext, false, 45),
                (dpapi, AccountFileKind::Dpapi, false, 320),
                (empty, AccountFileKind::Unrecognised, true, 0),
                (old, AccountFileKind::Unrecognised, true, 19),
            ]
        );
        let now = unix_seconds(SystemTime::now());
        let age = now - found[0].modified.unwrap();
        assert!((86_000..87_000).contains(&age), "{age}");
        // Nothing of a file's contents is in what comes back.
        assert!(!format!("{found:?}").contains("SECRETZZ"));
    }

    #[test]
    fn skips_directories_that_are_never_useful() {
        let tree = Tree::new("skipped");
        let kept = tree.file("Tools/AccountData.json", b"[]", 1);
        let in_appdata = tree.file("AppData/Local/RAM/AccountData.json", b"[]", 2);
        let temp_elsewhere = tree.file("Local/Temp/AccountData.json", b"[]", 3);
        for dir in [
            "node_modules/pkg",
            ".git",
            ".hidden",
            "Windows/System32",
            "Program Files/RAM",
            "PROGRAM FILES (X86)/RAM",
            "ProgramData",
            "$Recycle.Bin",
            "System Volume Information",
            "WinSxS",
            ".cargo",
            ".rustup",
            "project/target/debug",
            "AppData/Local/Temp/x",
            "appdata/local/TEMP",
        ] {
            tree.file(&format!("{dir}/AccountData.json"), b"[]", 0);
        }

        let found = find_account_files(std::slice::from_ref(&tree.0), 8, LONG);
        let paths: Vec<_> = found.iter().map(|f| f.path.clone()).collect();
        assert_eq!(paths, vec![kept, in_appdata, temp_elsewhere]);

        // A root is entered whatever it is called.
        let root = tree.0.join("project").join("target");
        let found = find_account_files(std::slice::from_ref(&root), 8, LONG);
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("debug/AccountData.json"));
    }

    #[test]
    fn depth_limit_counts_from_the_root() {
        let tree = Tree::new("depth");
        let d0 = tree.file("AccountData.json", b"[]", 1);
        let d1 = tree.file("one/AccountData.json", b"[]", 2);
        let d2 = tree.file("one/two/AccountData.json", b"[]", 3);
        let d3 = tree.file("one/two/three/AccountData.json", b"[]", 4);
        let roots = [tree.0.clone()];
        let paths = |depth| find_account_files(&roots, depth, LONG).into_iter().map(|f| f.path).collect::<Vec<_>>();
        assert_eq!(paths(0), vec![d0.clone()]);
        assert_eq!(paths(1), vec![d0.clone(), d1.clone()]);
        assert_eq!(paths(2), vec![d0.clone(), d1.clone(), d2.clone()]);
        assert_eq!(paths(9), vec![d0, d1, d2, d3]);
    }

    #[test]
    fn overlapping_roots_report_a_file_once() {
        let tree = Tree::new("roots");
        let on_desktop = tree.file("Desktop/RAM/AccountData.json", b"[]", 1);
        let deep = tree.file("stuff/a/b/AccountData.json", b"[]", 2);

        let roots = default_roots(&tree.0);
        assert_eq!(roots, vec![tree.0.join("Desktop"), tree.0.join("Downloads"), tree.0.join("Documents"), tree.0.clone()]);
        // Downloads and Documents do not exist here; that is not an error.
        let found = find_account_files(&roots, 3, LONG);
        assert_eq!(found.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), vec![on_desktop.clone(), deep]);

        // Reached through its own root, the Desktop keeps that root's depth.
        let found = find_account_files(&roots, 1, LONG);
        assert_eq!(found.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), vec![on_desktop]);
        assert!(find_account_files(&[], 3, LONG).is_empty());
    }

    #[test]
    fn returns_at_most_fifty_and_keeps_the_newest() {
        let tree = Tree::new("many");
        for i in 0..(MAX_FOUND as u64 + 5) {
            tree.file(&format!("copy {i:02}/AccountData.json"), b"[]", i + 1);
        }
        let found = find_account_files(std::slice::from_ref(&tree.0), 2, LONG);
        assert_eq!(found.len(), MAX_FOUND);
        assert!(found[0].path.ends_with("copy 00/AccountData.json"));
        assert!(found[MAX_FOUND - 1].path.ends_with("copy 49/AccountData.json"));
        assert!(found.windows(2).all(|pair| pair[0].modified >= pair[1].modified));
        assert!(found.iter().all(|f| f.kind == AccountFileKind::Plaintext));
    }

    #[test]
    fn a_spent_budget_returns_at_once() {
        let tree = Tree::new("budget");
        tree.file("AccountData.json", b"[]", 1);
        for i in 0..40 {
            tree.file(&format!("dir {i}/AccountData.json"), b"[]", 1);
        }
        let started = Instant::now();
        let found = find_account_files(std::slice::from_ref(&tree.0), 8, Duration::ZERO);
        assert!(found.is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
        // With time to spend, the same call finds them all.
        assert_eq!(find_account_files(std::slice::from_ref(&tree.0), 8, LONG).len(), 41);
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_not_followed() {
        let tree = Tree::new("junction");
        let real = tree.file("real/AccountData.json", b"[]", 1);
        let link = tree.0.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(tree.0.join("real"))
            .output()
            .is_ok_and(|out| out.status.success());
        if !made {
            return;
        }
        assert!(link.join("AccountData.json").is_file(), "the junction resolves");
        let found = find_account_files(std::slice::from_ref(&tree.0), 4, LONG);
        assert_eq!(found.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), vec![real]);
    }
}
