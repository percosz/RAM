//! Account search and ordering.
//!
//! A query is a list of whitespace-separated terms that must all match.
//! Plain terms match username, alias, tags, notes and group. Prefixed terms
//! narrow one field: `tag:`, `group:`, `user:`, `note:`, and the flags
//! `is:fav`, `is:main`, `is:running`.

use crate::model::{Account, AccountId, Store};

/// Lower-cased text for one account, built once and reused for every
/// keystroke.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub id: AccountId,
    names: String,
    tags: Vec<String>,
    group: String,
    notes: String,
    favorite: bool,
}

#[derive(Debug, Default, Clone)]
pub struct SearchIndex {
    entries: Vec<IndexEntry>,
}

impl SearchIndex {
    pub fn build(accounts: &[Account]) -> Self {
        SearchIndex {
            entries: accounts
                .iter()
                .map(|a| IndexEntry {
                    id: a.id,
                    names: format!("{}\n{}", a.username.to_lowercase(), a.alias.to_lowercase()),
                    tags: a.tags.iter().map(|t| t.to_lowercase()).collect(),
                    group: a.group.to_lowercase(),
                    notes: a.notes.to_lowercase(),
                    favorite: a.favorite,
                })
                .collect(),
        }
    }

    /// IDs of matching accounts in index order.
    pub fn query(&self, query: &str, ctx: &QueryContext) -> Vec<AccountId> {
        let terms = parse(query);
        self.entries
            .iter()
            .filter(|e| terms.iter().all(|t| matches(e, t, ctx)))
            .map(|e| e.id)
            .collect()
    }
}

/// State that lives outside the account record.
#[derive(Debug, Default, Clone)]
pub struct QueryContext {
    pub main: Option<AccountId>,
    pub running: Vec<AccountId>,
}

#[derive(Debug, PartialEq, Eq)]
enum Term {
    Any(String),
    Tag(String),
    Group(String),
    User(String),
    Note(String),
    Favorite,
    Main,
    Running,
}

fn parse(query: &str) -> Vec<Term> {
    query
        .split_whitespace()
        .map(|raw| {
            let w = raw.to_lowercase();
            if let Some(v) = w.strip_prefix("tag:") {
                Term::Tag(v.to_string())
            } else if let Some(v) = w.strip_prefix("group:") {
                Term::Group(v.to_string())
            } else if let Some(v) = w.strip_prefix("user:") {
                Term::User(v.to_string())
            } else if let Some(v) = w.strip_prefix("note:") {
                Term::Note(v.to_string())
            } else {
                match w.as_str() {
                    "is:fav" | "is:favorite" | "is:favourite" => Term::Favorite,
                    "is:main" => Term::Main,
                    "is:running" => Term::Running,
                    _ => Term::Any(w),
                }
            }
        })
        .collect()
}

fn matches(e: &IndexEntry, term: &Term, ctx: &QueryContext) -> bool {
    match term {
        Term::Any(t) => {
            e.names.contains(t.as_str())
                || e.group.contains(t.as_str())
                || e.notes.contains(t.as_str())
                || e.tags.iter().any(|x| x.contains(t.as_str()))
        }
        Term::Tag(t) => e.tags.iter().any(|x| x.contains(t.as_str())),
        Term::Group(t) => e.group.contains(t.as_str()),
        Term::User(t) => e.names.contains(t.as_str()),
        Term::Note(t) => e.notes.contains(t.as_str()),
        Term::Favorite => e.favorite,
        Term::Main => ctx.main == Some(e.id),
        Term::Running => ctx.running.contains(&e.id),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Main first, then favorites, then group, then name.
    #[default]
    Default,
    Name,
    Username,
    Group,
    SignIn,
    LastUsed,
}

/// Sorts account IDs for display. Main is always first so it cannot be lost
/// in a long list.
pub fn sort_ids(store: &Store, ids: &mut [AccountId], key: SortKey, descending: bool) {
    // Sort keys are computed once per account, so the cost stays
    // O(n log n) for large lists.
    struct Rec {
        not_main: bool,
        n: i64,
        a: String,
        b: String,
    }
    let by_id: std::collections::HashMap<AccountId, &Account> = store.accounts.iter().map(|a| (a.id, a)).collect();
    let reverse = descending && key != SortKey::Default;
    ids.sort_by_cached_key(|id| {
        let Some(acc) = by_id.get(id) else {
            return (true, i64::MAX, String::new(), String::new());
        };
        let name = acc.display_name().to_lowercase();
        let rec = match key {
            SortKey::Default => Rec { not_main: false, n: !acc.favorite as i64, a: acc.group.to_lowercase(), b: name },
            SortKey::Name => Rec { not_main: false, n: 0, a: name, b: String::new() },
            SortKey::Username => Rec { not_main: false, n: 0, a: acc.username.to_lowercase(), b: String::new() },
            SortKey::Group => Rec { not_main: false, n: 0, a: acc.group.to_lowercase(), b: name },
            SortKey::SignIn => Rec { not_main: false, n: acc.sign_in.state as i64, a: name, b: String::new() },
            // Most recent first; never-used accounts last.
            SortKey::LastUsed => Rec { not_main: false, n: acc.last_used.map(|t| -t).unwrap_or(i64::MAX), a: name, b: String::new() },
        };
        (rec.not_main || !store.is_main(acc.id), rec.n, rec.a, rec.b)
    });
    if reverse {
        // Main stays first; everything after it is reversed.
        let split = ids.iter().position(|id| !store.is_main(*id)).unwrap_or(ids.len());
        ids[split..].reverse();
    }
}
/// Accounts a batch action applies to: the given selection minus Main,
/// unless Main is explicitly included by the setting.
pub fn batch_targets(store: &Store, selection: &[AccountId]) -> Vec<AccountId> {
    selection
        .iter()
        .copied()
        .filter(|id| store.settings.include_main_in_batch || !store.is_main(*id))
        .collect()
}

/// IDs chosen by Select All from the visible rows.
pub fn select_all(store: &Store, visible: &[AccountId]) -> Vec<AccountId> {
    batch_targets(store, visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let mut s = Store::default();
        let mut add = |name: &str, alias: &str, group: &str, tags: &[&str], notes: &str, fav: bool| {
            let id = s.alloc_id();
            s.accounts.push(Account {
                id,
                username: name.into(),
                alias: alias.into(),
                group: group.into(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                notes: notes.into(),
                favorite: fav,
                ..Default::default()
            });
            id
        };
        add("MainGuy", "", "Mains", &["ultra"], "primary", true);
        add("alt_one", "Farmer", "Alts", &["farm", "gaia"], "silver stash\nsecond line", false);
        add("alt_two", "", "Alts", &["khei"], "", true);
        s.main_account = Some(1);
        s
    }

    #[test]
    fn matches_username_alias_tag_notes_and_group() {
        let s = store();
        let idx = SearchIndex::build(&s.accounts);
        let ctx = QueryContext::default();
        assert_eq!(idx.query("farmer", &ctx), vec![2]);
        assert_eq!(idx.query("ALT_", &ctx), vec![2, 3]);
        assert_eq!(idx.query("gaia", &ctx), vec![2]);
        assert_eq!(idx.query("second", &ctx), vec![2]);
        assert_eq!(idx.query("alts", &ctx), vec![2, 3]);
        assert_eq!(idx.query("", &ctx), vec![1, 2, 3]);
    }

    #[test]
    fn all_terms_must_match_and_prefixes_narrow() {
        let s = store();
        let idx = SearchIndex::build(&s.accounts);
        let ctx = QueryContext { main: Some(1), running: vec![3] };
        assert_eq!(idx.query("alt khei", &ctx), vec![3]);
        assert_eq!(idx.query("tag:farm", &ctx), vec![2]);
        assert_eq!(idx.query("group:alts is:fav", &ctx), vec![3]);
        assert_eq!(idx.query("is:main", &ctx), vec![1]);
        assert_eq!(idx.query("is:running", &ctx), vec![3]);
        assert_eq!(idx.query("note:silver", &ctx), vec![2]);
        assert!(idx.query("tag:alts", &ctx).is_empty());
    }

    #[test]
    fn main_sorts_first_and_is_excluded_from_select_all() {
        let mut s = store();
        let mut ids = vec![3, 2, 1];
        sort_ids(&s, &mut ids, SortKey::Name, true);
        assert_eq!(ids[0], 1);
        assert_eq!(select_all(&s, &ids).len(), 2);
        assert!(!select_all(&s, &ids).contains(&1));
        assert_eq!(batch_targets(&s, &[1, 2]), vec![2]);
        s.settings.include_main_in_batch = true;
        assert!(select_all(&s, &ids).contains(&1));
    }

    #[test]
    fn hundred_accounts_query_is_fast() {
        let mut s = Store::default();
        for i in 0..100 {
            let id = s.alloc_id();
            s.accounts.push(Account {
                id,
                username: format!("account_{i:03}"),
                notes: "some longer note text ".repeat(20),
                ..Default::default()
            });
        }
        let idx = SearchIndex::build(&s.accounts);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(idx.query("account_05 note", &QueryContext::default()).len(), 10);
        }
        // 1000 queries; generous bound so slow CI machines pass.
        assert!(start.elapsed().as_millis() < 2000);
    }
}
