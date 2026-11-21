//! Export accounts to the legacy Roblox Account Manager `AccountData.json`.
//!
//! Writes RAM's accounts back out in the exact legacy shape so the legacy app,
//! or another manager, can import them. Only non-secret metadata is written by
//! default; a session cookie is included only when the caller opts in and
//! supplies it from RAM's encrypted vault. The output round-trips through
//! [`crate::import`]: its detection, decryption and field mapping are the
//! inverse of what this module produces.

use crate::import::LEGACY_DPAPI_ENTROPY;
use crate::model::{AccountId, Store, Target};
use serde::Serialize;
use std::collections::BTreeMap;

/// The legacy group given to an account with no RAM group.
const LEGACY_DEFAULT_GROUP: &str = "Default";

/// The legacy "never" timestamp (`DateTime.MinValue`), written when an account
/// has never been used.
const NEVER: &str = "0001-01-01T00:00:00";

/// How the exported `AccountData.json` is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportEncryption {
    /// A plain UTF-8 JSON array, first byte `[`. The legacy app only reads this
    /// when its `NoEncryption.IUnderstandTheRisks` marker file exists; the
    /// summary carries a warning saying so.
    Plaintext,
    /// Encrypted with Windows DPAPI via the supplied protector (the legacy
    /// default format).
    Dpapi,
}

/// Protects a payload with Windows DPAPI. Implemented by the Windows layer; it
/// is the inverse of [`crate::import::LegacyDecryptor`].
pub trait LegacyProtector {
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String>;
}

/// What to export and how.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub encryption: ExportEncryption,
    /// When true, include each account's `.ROBLOSECURITY` cookie as
    /// `SecurityToken`.
    pub include_cookies: bool,
}

/// Why an export could not be produced. Never carries a cookie.
#[derive(Debug)]
pub enum ExportError {
    /// `Dpapi` was requested without a protector.
    NoProtector,
    /// The protector failed; the string is its own message.
    Protect(String),
    /// Serializing the account array failed.
    Serialize(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::NoProtector => f.write_str("DPAPI export needs a protector"),
            ExportError::Protect(why) => write!(f, "DPAPI protection failed: {why}"),
            ExportError::Serialize(why) => write!(f, "could not serialize accounts: {why}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// What an export produced. `warnings` and the counts are safe to display;
/// none of them contains a cookie.
#[derive(Debug, Default)]
pub struct ExportSummary {
    pub accounts: u32,
    pub with_cookie: u32,
    pub bytes: usize,
    pub warnings: Vec<String>,
}

/// One account as the legacy app serializes it. Field order and names match
/// `Classes/Account.cs`; `Password` is always empty and `BrowserTrackerID` is
/// omitted, since RAM stores neither.
#[derive(Serialize)]
struct LegacyAccountOut {
    #[serde(rename = "Valid")]
    valid: bool,
    #[serde(rename = "SecurityToken")]
    security_token: String,
    #[serde(rename = "Username")]
    username: String,
    #[serde(rename = "LastUse")]
    last_use: String,
    #[serde(rename = "Alias")]
    alias: String,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Password")]
    password: String,
    #[serde(rename = "Group")]
    group: String,
    #[serde(rename = "UserID")]
    user_id: i64,
    #[serde(rename = "Fields")]
    fields: BTreeMap<&'static str, String>,
    #[serde(rename = "BrowserTrackerID", skip_serializing_if = "Option::is_none")]
    browser_tracker_id: Option<String>,
}

/// Serializes the given accounts to `AccountData.json` bytes.
///
/// `cookie_of` returns an account's cookie when `include_cookies` is set; a
/// `None` or empty cookie writes an empty `SecurityToken`. `store` supplies the
/// account records and the recent-launch history used to fill each account's
/// `Fields.SavedPlaceId`/`SavedJobId`. Account ids not present in the store are
/// skipped.
pub fn export(
    store: &Store,
    accounts: &[AccountId],
    cookie_of: &dyn Fn(AccountId) -> Option<String>,
    opts: &ExportOptions,
    protector: Option<&dyn LegacyProtector>,
) -> Result<(Vec<u8>, ExportSummary), ExportError> {
    // Fail before doing any work if DPAPI was asked for without a protector.
    if opts.encryption == ExportEncryption::Dpapi && protector.is_none() {
        return Err(ExportError::NoProtector);
    }

    let mut summary = ExportSummary::default();
    let mut rows: Vec<LegacyAccountOut> = Vec::with_capacity(accounts.len());

    for &id in accounts {
        let Some(account) = store.account(id) else { continue };
        summary.accounts += 1;

        let security_token = if opts.include_cookies {
            match cookie_of(id) {
                Some(cookie) if !cookie.is_empty() => {
                    summary.with_cookie += 1;
                    cookie
                }
                _ => String::new(),
            }
        } else {
            String::new()
        };

        let group = if account.group.trim().is_empty() {
            LEGACY_DEFAULT_GROUP.to_string()
        } else {
            account.group.clone()
        };

        rows.push(LegacyAccountOut {
            valid: true,
            security_token,
            username: account.username.clone(),
            last_use: account.last_used.map_or_else(|| NEVER.to_string(), format_iso8601),
            alias: account.alias.clone(),
            description: account.notes.clone(),
            password: String::new(),
            group,
            user_id: account.roblox_user_id.and_then(|id| i64::try_from(id).ok()).unwrap_or(0),
            fields: saved_fields(store, id),
            browser_tracker_id: None,
        });
    }

    let bytes = match opts.encryption {
        ExportEncryption::Plaintext => {
            summary.warnings.push(
                "This file is unencrypted. The legacy app only reads it when its \
                 \"NoEncryption.IUnderstandTheRisks\" marker file exists."
                    .to_string(),
            );
            serde_json::to_vec_pretty(&rows).map_err(|e| ExportError::Serialize(e.to_string()))?
        }
        ExportEncryption::Dpapi => {
            let json = serde_json::to_vec(&rows).map_err(|e| ExportError::Serialize(e.to_string()))?;
            // Checked above, so the protector is present here.
            let protector = protector.ok_or(ExportError::NoProtector)?;
            protector.protect(&json, LEGACY_DPAPI_ENTROPY).map_err(ExportError::Protect)?
        }
    };

    summary.bytes = bytes.len();
    Ok((bytes, summary))
}

/// Builds the legacy `Fields` map for an account from its most recent launch.
/// Only the saved place and server keys are written; a target the legacy format
/// cannot express (a share link, with no place ID) yields an empty map.
fn saved_fields(store: &Store, id: AccountId) -> BTreeMap<&'static str, String> {
    let mut fields = BTreeMap::new();
    let Some(recent) = store.recent.iter().filter(|r| r.account == id).max_by_key(|r| r.at) else {
        return fields;
    };
    match &recent.target {
        Target::Place { place_id } => {
            fields.insert("SavedPlaceId", place_id.to_string());
        }
        Target::LiveServer { place_id, job_id } => {
            fields.insert("SavedPlaceId", place_id.to_string());
            fields.insert("SavedJobId", job_id.clone());
        }
        Target::PrivateServer { place_id, link_code } => {
            fields.insert("SavedPlaceId", place_id.to_string());
            fields.insert(
                "SavedJobId",
                format!("https://www.roblox.com/games/{place_id}?privateServerLinkCode={link_code}"),
            );
        }
        // A share link has no place ID, so the legacy place/job fields cannot
        // represent it; the importer could not rebuild it either.
        Target::ShareLink { .. } => {}
    }
    fields
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SS` in UTC, no offset. This is the
/// inverse of `import::parse_iso8601`, which reads a missing offset as UTC, so
/// the pair round-trips to the same second.
fn format_iso8601(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs = unix_secs.rem_euclid(86_400);
    let (hour, minute, second) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{self, AccountFileKind, LegacyDecryptor};
    use crate::model::{Account, RecentLaunch};
    use std::path::Path;

    const DPAPI_MAGIC: [u8; 20] = [
        0x01, 0x00, 0x00, 0x00, 0xD0, 0x8C, 0x9D, 0xDF, 0x01, 0x15, 0xD1, 0x11, 0x8C, 0x7A, 0x00, 0xC0, 0x4F, 0xC2,
        0x97, 0xEB,
    ];
    const COOKIE: &str = "_|WARNING:-DO-NOT-SHARE-THIS.--COOKIEZZ-DISTINCT-4417";
    const NOW: i64 = 1_800_000_000;

    fn account(id: AccountId, username: &str) -> Account {
        Account { id, username: username.to_string(), ..Account::default() }
    }

    /// A store with three accounts: a full one in a group, a plain ungrouped
    /// one, and one with notes but no user id.
    fn sample_store() -> (Store, Vec<AccountId>) {
        let mut store = Store::default();
        store.accounts.push(Account {
            alias: "Main farm".to_string(),
            roblox_user_id: Some(1001),
            group: "Farm".to_string(),
            notes: "Trades on weekends".to_string(),
            last_used: Some(NOW),
            ..account(1, "AlphaFarmer")
        });
        store.accounts.push(Account { roblox_user_id: Some(1002), ..account(2, "BetaAlt") });
        store.accounts.push(Account {
            alias: "Gamma".to_string(),
            notes: "no id".to_string(),
            ..account(3, "GammaNoId")
        });
        (store, vec![1, 2, 3])
    }

    fn plaintext_opts(include_cookies: bool) -> ExportOptions {
        ExportOptions { encryption: ExportEncryption::Plaintext, include_cookies }
    }

    /// Reads exported bytes back through the importer as `AccountData.json`.
    fn scan_back(bytes: &[u8], decryptor: Option<&dyn LegacyDecryptor>) -> import::Scan {
        let owned = bytes.to_vec();
        import::scan_files(
            Path::new("C:/export"),
            &|name| (name == "AccountData.json").then(|| owned.clone()),
            decryptor,
        )
    }

    #[test]
    fn round_trips_metadata_without_cookies() {
        let (store, ids) = sample_store();
        let (bytes, summary) = export(&store, &ids, &|_| None, &plaintext_opts(false), None).unwrap();

        assert_eq!(bytes[0], b'[');
        assert_eq!(summary.accounts, 3);
        assert_eq!(summary.with_cookie, 0);
        assert_eq!(summary.bytes, bytes.len());
        // No cookie was ever requested, so none can be in the output.
        assert!(!String::from_utf8_lossy(&bytes).contains("SecurityToken\": \"_|"));

        let scan = scan_back(&bytes, None);
        assert_eq!(scan.account_file, AccountFileKind::Plaintext);
        assert_eq!(scan.accounts.len(), 3);

        let alpha = &scan.accounts[0];
        assert_eq!(alpha.username, "AlphaFarmer");
        assert_eq!(alpha.alias, "Main farm");
        assert_eq!(alpha.description, "Trades on weekends");
        assert_eq!(alpha.group, "Farm");
        assert_eq!(alpha.user_id, Some(1001));
        assert_eq!(alpha.last_use, Some(NOW));

        let beta = &scan.accounts[1];
        assert_eq!(beta.username, "BetaAlt");
        assert_eq!(beta.alias, "");
        assert_eq!(beta.user_id, Some(1002));

        let gamma = &scan.accounts[2];
        assert_eq!(gamma.username, "GammaNoId");
        assert_eq!(gamma.alias, "Gamma");
        assert_eq!(gamma.description, "no id");
        assert_eq!(gamma.user_id, None);
    }

    #[test]
    fn plaintext_export_warns_about_the_marker_file() {
        let (store, ids) = sample_store();
        let (_, summary) = export(&store, &ids, &|_| None, &plaintext_opts(false), None).unwrap();
        assert_eq!(summary.warnings.len(), 1);
        assert!(summary.warnings[0].contains("NoEncryption"));
    }

    #[test]
    fn empty_group_exports_as_default_and_imports_back_to_empty() {
        // Account 2 (BetaAlt) is ungrouped.
        let mut store = Store::default();
        store.accounts.push(account(2, "BetaAlt"));
        let (bytes, _) = export(&store, &[2], &|_| None, &plaintext_opts(false), None).unwrap();

        // The legacy file carries "Default" for the ungrouped account.
        assert!(String::from_utf8_lossy(&bytes).contains("\"Default\""));

        // Importing turns "Default" back into an ungrouped account.
        let scan = scan_back(&bytes, None);
        let mut fresh = Store::default();
        let plan = import::plan(&fresh, &scan);
        import::apply(&mut fresh, &plan, NOW);
        let imported = fresh.accounts.iter().find(|a| a.username == "BetaAlt").unwrap();
        assert_eq!(imported.group, "");
    }

    #[test]
    fn non_empty_group_round_trips() {
        let mut store = Store::default();
        store.accounts.push(Account { group: "Farm".to_string(), ..account(1, "AlphaFarmer") });
        let (bytes, _) = export(&store, &[1], &|_| None, &plaintext_opts(false), None).unwrap();
        let scan = scan_back(&bytes, None);
        assert_eq!(scan.accounts[0].group, "Farm");
    }

    #[test]
    fn cookies_are_included_only_when_requested_and_only_where_present() {
        let (store, ids) = sample_store();
        // Only AlphaFarmer (id 1) has a cookie.
        let cookie_of = |id: AccountId| (id == 1).then(|| COOKIE.to_string());

        let (bytes, summary) = export(&store, &ids, &cookie_of, &plaintext_opts(true), None).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(summary.with_cookie, 1);
        assert!(text.contains(COOKIE));
        // Exactly one occurrence, and the other two tokens are empty.
        assert_eq!(text.matches(COOKIE).count(), 1);
        assert_eq!(text.matches("\"SecurityToken\": \"\"").count(), 2);

        // The importer never materializes SecurityToken, so the cookie never
        // reaches the scan.
        let scan = scan_back(&bytes, None);
        assert_eq!(scan.accounts.len(), 3);
    }

    #[test]
    fn cookies_are_absent_when_not_requested() {
        let (store, ids) = sample_store();
        let cookie_of = |_: AccountId| Some(COOKIE.to_string());
        let (bytes, summary) = export(&store, &ids, &cookie_of, &plaintext_opts(false), None).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(summary.with_cookie, 0);
        assert!(!text.contains(COOKIE));
        assert_eq!(text.matches("\"SecurityToken\": \"\"").count(), 3);
    }

    /// Prepends the real DPAPI magic bytes, as the OS protector would.
    struct FakeProtector;
    impl LegacyProtector for FakeProtector {
        fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
            assert_eq!(entropy, LEGACY_DPAPI_ENTROPY);
            let mut blob = DPAPI_MAGIC.to_vec();
            blob.extend_from_slice(data);
            Ok(blob)
        }
    }

    /// Strips the magic bytes the fake protector added.
    struct FakeDecryptor;
    impl LegacyDecryptor for FakeDecryptor {
        fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
            assert_eq!(entropy, LEGACY_DPAPI_ENTROPY);
            data.strip_prefix(&DPAPI_MAGIC[..]).map(<[u8]>::to_vec).ok_or_else(|| "no magic".to_string())
        }
    }

    #[test]
    fn dpapi_round_trips_through_the_importer() {
        let (store, ids) = sample_store();
        let opts = ExportOptions { encryption: ExportEncryption::Dpapi, include_cookies: false };
        let (bytes, summary) = export(&store, &ids, &|_| None, &opts, Some(&FakeProtector)).unwrap();

        assert_eq!(summary.bytes, bytes.len());
        assert_eq!(import::detect_account_file(&bytes), AccountFileKind::Dpapi);

        let scan = scan_back(&bytes, Some(&FakeDecryptor));
        assert_eq!(scan.account_file, AccountFileKind::Dpapi);
        assert_eq!(scan.accounts.len(), 3);
        assert_eq!(scan.accounts[0].username, "AlphaFarmer");
        assert_eq!(scan.accounts[0].user_id, Some(1001));
    }

    #[test]
    fn dpapi_without_a_protector_is_an_error() {
        let (store, ids) = sample_store();
        let opts = ExportOptions { encryption: ExportEncryption::Dpapi, include_cookies: false };
        assert!(matches!(export(&store, &ids, &|_| None, &opts, None), Err(ExportError::NoProtector)));
    }

    #[test]
    fn detect_recognizes_both_output_formats() {
        let (store, ids) = sample_store();
        let (plain, _) = export(&store, &ids, &|_| None, &plaintext_opts(false), None).unwrap();
        assert_eq!(import::detect_account_file(&plain), AccountFileKind::Plaintext);

        let opts = ExportOptions { encryption: ExportEncryption::Dpapi, include_cookies: false };
        let (enc, _) = export(&store, &ids, &|_| None, &opts, Some(&FakeProtector)).unwrap();
        assert_eq!(import::detect_account_file(&enc), AccountFileKind::Dpapi);
    }

    #[test]
    fn never_used_accounts_get_the_legacy_min_date() {
        let mut store = Store::default();
        store.accounts.push(account(1, "AlphaFarmer"));
        let (bytes, _) = export(&store, &[1], &|_| None, &plaintext_opts(false), None).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains(NEVER));
    }

    #[test]
    fn last_use_round_trips_to_the_same_second() {
        for secs in [0_i64, 1, 1_000_000_000, NOW, 1_700_000_000, 1_900_000_123] {
            let mut store = Store::default();
            store.accounts.push(Account { last_used: Some(secs), ..account(1, "AlphaFarmer") });
            let (bytes, _) = export(&store, &[1], &|_| None, &plaintext_opts(false), None).unwrap();
            let scan = scan_back(&bytes, None);
            assert_eq!(scan.accounts[0].last_use, Some(secs), "secs={secs}");
        }
    }

    #[test]
    fn saved_fields_come_from_the_most_recent_launch() {
        let mut store = Store::default();
        store.accounts.push(account(1, "AlphaFarmer"));
        store.push_recent(RecentLaunch { account: 1, target: Target::Place { place_id: 606_849_621 }, at: NOW });
        let (bytes, _) = export(&store, &[1], &|_| None, &plaintext_opts(false), None).unwrap();

        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\"SavedPlaceId\": \"606849621\""));

        // The saved place comes back as a destination in the import plan.
        let scan = scan_back(&bytes, None);
        assert_eq!(scan.accounts[0].saved, Some(Target::Place { place_id: 606_849_621 }));
    }

    #[test]
    fn unknown_account_ids_are_skipped() {
        let (store, _) = sample_store();
        let (bytes, summary) = export(&store, &[1, 999], &|_| None, &plaintext_opts(false), None).unwrap();
        assert_eq!(summary.accounts, 1);
        let scan = scan_back(&bytes, None);
        assert_eq!(scan.accounts.len(), 1);
    }
}
