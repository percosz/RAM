//! The Destinations view: the table of games and saved destinations, adding
//! a destination from a pasted link, and the recent launch history.

use crate::app::{self, now, tone_color, App, Tone, LABEL_MUTED};
use crate::dialogs::{self, Field};
use crate::ui;
use ram_core::links;
use ram_core::model::{Destination, DestinationId, Durability, Game, RecentLaunch, Store, Target};
use ram_core::platform::BrowserApi;
use ram_core::search;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_DELETE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_LIST: u16 = 1100;
const ID_RECENT: u16 = 1101;
const ID_LINK: u16 = 1102;
const ID_NAME: u16 = 1103;
const ID_ADD: u16 = 1104;
const ID_LAUNCH: u16 = 1105;
const ID_RENAME: u16 = 1106;
const ID_NOTES: u16 = 1107;
const ID_REMOVE: u16 = 1108;
const ID_DEFAULT: u16 = 1109;
const ID_PIN: u16 = 1110;
const ID_PAGE: u16 = 1111;
const ID_AGAIN: u16 = 1112;
const ID_CLEAR: u16 = 1113;

/// Saved table columns: Name, Game, Type, Last used, Notes. Name and Notes
/// share the width the fixed columns leave.
const COLUMNS: [(&str, i32); 5] = [("Name", 200), ("Game", 190), ("Type", 190), ("Last used", 84), ("Notes", 160)];
/// Recent table columns: Account, Destination, Type, When. The first two
/// share the width the fixed columns leave.
const RECENT_COLUMNS: [(&str, i32); 4] = [("Account", 200), ("Destination", 300), ("Type", 190), ("When", 84)];

/// One row of the saved table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A game from `store.games`, by place ID.
    Game(u64),
    /// A saved destination.
    Saved(DestinationId),
}

#[derive(Default)]
pub struct DestinationsView {
    pub list: HWND,
    link: HWND,
    name: HWND,
    add: HWND,
    hint: HWND,
    launch: HWND,
    rename: HWND,
    notes: HWND,
    remove: HWND,
    default: HWND,
    pin: HWND,
    page: HWND,
    recent_label: HWND,
    recent: HWND,
    again: HWND,
    clear: HWND,
    /// Saved table rows in display order; the virtual list reads from this.
    rows: Vec<Row>,
    /// Set while the view updates its own controls, so the change
    /// notifications those updates raise are ignored.
    loading: bool,
    /// DPI the fixed column widths were last set for.
    column_dpi: u32,
    /// Row height last given to the recent list.
    recent_row_height: i32,
}

pub fn create(app: &mut App) {
    let p = app.panels[1];
    let v = &mut app.destinations;
    v.link = ui::edit(p, ID_LINK);
    ui::set_cue(v.link, "Paste a Roblox link or place ID");
    v.name = ui::edit(p, ID_NAME);
    ui::set_cue(v.name, "Name (optional)");
    v.add = ui::button(p, ID_ADD, "Add");
    v.hint = ui::label(p, LABEL_MUTED, "");
    v.list = ui::listview(p, ID_LIST, ui::LIST_STYLE_SINGLE);
    for (i, (title, width)) in COLUMNS.iter().enumerate() {
        ui::list_add_column(v.list, i as i32, title, app.m.px(*width));
    }
    crate::theme::subclass_list(v.list);
    v.launch = ui::button(p, ID_LAUNCH, "Launch selected accounts");
    v.rename = ui::button(p, ID_RENAME, "Rename\u{2026}");
    v.notes = ui::button(p, ID_NOTES, "Edit notes\u{2026}");
    v.remove = ui::button(p, ID_REMOVE, "Remove");
    v.default = ui::button(p, ID_DEFAULT, "Set as default");
    v.pin = ui::button(p, ID_PIN, "Pin");
    v.page = ui::button(p, ID_PAGE, "Open game page");
    v.recent_label = ui::label(p, LABEL_MUTED, "Recent launches");
    v.recent = ui::listview(p, ID_RECENT, ui::LIST_STYLE_SINGLE);
    for (i, (title, width)) in RECENT_COLUMNS.iter().enumerate() {
        ui::list_add_column(v.recent, i as i32, title, app.m.px(*width));
    }
    crate::theme::subclass_list(v.recent);
    v.again = ui::button(p, ID_AGAIN, "Launch again");
    v.clear = ui::button(p, ID_CLEAR, "Clear history");
    v.column_dpi = app.m.dpi;

    enter_sends(v.link, ID_ADD);
    enter_sends(v.name, ID_ADD);
    enter_sends(v.list, ID_LAUNCH);
    enter_sends(v.recent, ID_AGAIN);
}

/// Makes Enter in `ctl` send `command` to the panel. Without this the
/// dialog manager consumes the key before the control sees it.
fn enter_sends(ctl: HWND, command: u16) {
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        command: usize,
    ) -> LRESULT {
        let enter = wparam.0 == VK_RETURN.0 as usize;
        match msg {
            WM_GETDLGCODE if lparam.0 != 0 => {
                let code = DefSubclassProc(hwnd, msg, wparam, lparam);
                let pending = &*(lparam.0 as *const MSG);
                let key = pending.message == WM_KEYDOWN || pending.message == WM_CHAR;
                if key && pending.wParam.0 == VK_RETURN.0 as usize {
                    LRESULT(code.0 | DLGC_WANTALLKEYS as isize)
                } else {
                    code
                }
            }
            WM_KEYDOWN if enter => {
                if let Ok(parent) = GetParent(hwnd) {
                    ui::post(parent, WM_COMMAND, command, 0);
                }
                LRESULT(0)
            }
            // The character that follows the key press would only beep.
            WM_CHAR if enter => LRESULT(0),
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
    unsafe {
        let _ = SetWindowSubclass(ctl, Some(proc), 2, command as usize);
    }
}

pub fn layout(app: &mut App) {
    let rc = ui::client_rect(app.panels[1]);
    let m = &app.m;
    let v = &app.destinations;
    let (gap, row) = (m.gap(), m.row());
    let lh = m.px(18);
    let w = rc.right - 2 * gap;

    // Top: link box, name box, Add, and the validation line under them.
    let add_w = m.px(84);
    let name_w = m.px(220).min(w / 3);
    let link_w = w - name_w - add_w - 2 * gap;
    ui::place(v.link, gap, 0, link_w, row);
    ui::place(v.name, gap + link_w + gap, 0, name_w, row);
    ui::place(v.add, gap + link_w + name_w + 2 * gap, 0, add_w, row);
    let mut y = row + m.px(3);
    ui::place(v.hint, gap, y, w, lh);
    y += lh + gap;

    // The two tables share the height the fixed rows leave.
    let fixed = gap + row + gap + lh + m.px(2) + gap + row + gap;
    let tables = (rc.bottom - y - fixed).max(0);
    let saved_h = tables * 11 / 20;
    let recent_h = tables - saved_h;

    ui::place(v.list, gap, y, w, saved_h);
    y += saved_h + gap;
    let launch_w = m.px(172);
    ui::place(v.launch, gap, y, launch_w, row);
    let mut x = gap + launch_w + gap;
    // Widths fit each label; together they fit the minimum window width.
    for (b, dip) in [(v.rename, 92), (v.notes, 100), (v.remove, 84), (v.default, 110), (v.pin, 72), (v.page, 120)] {
        ui::place(b, x, y, m.px(dip), row);
        x += m.px(dip) + gap;
    }
    y += row + gap;

    ui::place(v.recent_label, gap, y, w, lh);
    y += lh + m.px(2);
    ui::place(v.recent, gap, y, w, recent_h);
    y += recent_h + gap;
    let recent_button_w = m.px(112);
    ui::place(v.again, gap, y, recent_button_w, row);
    ui::place(v.clear, gap + recent_button_w + gap, y, recent_button_w, row);

    // Fixed columns follow the DPI; flexible ones follow the width.
    let rescale = v.column_dpi != m.dpi;
    let room = w - m.px(24);
    let set = |list: HWND, column: usize, width: i32| {
        ui::send(list, LVM_SETCOLUMNWIDTH, column, width as isize);
    };
    if rescale {
        for i in [1, 2, 3] {
            set(v.list, i, m.px(COLUMNS[i].1));
        }
        for i in [2, 3] {
            set(v.recent, i, m.px(RECENT_COLUMNS[i].1));
        }
    }
    let flexible = room - m.px(COLUMNS[1].1 + COLUMNS[2].1 + COLUMNS[3].1);
    let name_col = (flexible * 9 / 20).clamp(m.px(140), m.px(320));
    set(v.list, 0, name_col);
    set(v.list, 4, (flexible - name_col).max(m.px(100)));
    let flexible = room - m.px(RECENT_COLUMNS[2].1 + RECENT_COLUMNS[3].1);
    let account_col = (flexible * 2 / 5).max(m.px(140));
    set(v.recent, 0, account_col);
    set(v.recent, 1, (flexible - account_col).max(m.px(160)));

    let (dpi, row_height) = (m.dpi, m.list_row());
    let v = &mut app.destinations;
    v.column_dpi = dpi;
    if v.recent_row_height != row_height {
        v.recent_row_height = row_height;
        ui::list_set_row_height(v.recent, row_height);
    }
}

// ---- data ------------------------------------------------------------------

/// Pinned games first (the built-in entry leads `store.games`), then saved
/// destinations by name, then the other games.
fn build_rows(store: &Store) -> Vec<Row> {
    let mut rows: Vec<Row> = store.games.iter().filter(|g| g.pinned).map(|g| Row::Game(g.place_id)).collect();
    rows.sort_by_key(|r| !matches!(r, Row::Game(id) if store.games.iter().any(|g| g.place_id == *id && g.builtin)));
    let mut saved: Vec<&Destination> = store.destinations.iter().collect();
    saved.sort_by_key(|d| d.name.to_lowercase());
    rows.extend(saved.into_iter().map(|d| Row::Saved(d.id)));
    rows.extend(store.games.iter().filter(|g| !g.pinned).map(|g| Row::Game(g.place_id)));
    rows
}

fn game(store: &Store, place_id: u64) -> Option<&Game> {
    store.games.iter().find(|g| g.place_id == place_id)
}

fn saved(store: &Store, id: DestinationId) -> Option<&Destination> {
    store.destinations.iter().find(|d| d.id == id)
}

/// Where a row launches to.
fn row_target(store: &Store, row: Row) -> Option<Target> {
    match row {
        Row::Game(place_id) => game(store, place_id).map(|g| Target::Place { place_id: g.place_id }),
        Row::Saved(id) => saved(store, id).map(|d| d.target.clone()),
    }
}

fn row_name(store: &Store, row: Row) -> String {
    match row {
        Row::Game(place_id) => store.game_name(place_id),
        Row::Saved(id) => saved(store, id).map(|d| d.name.clone()).unwrap_or_default(),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cell(store: &Store, row: Row, column: i32) -> String {
    match row {
        Row::Game(place_id) => {
            let Some(g) = game(store, place_id) else { return String::new() };
            let target = Target::Place { place_id };
            match column {
                0 => g.name.clone(),
                1 => links::describe(&target),
                2 => if g.pinned { "Pinned game" } else { "Game" }.to_string(),
                3 => ui::ago(now(), store.recent.iter().filter(|r| r.target == target).map(|r| r.at).max()),
                4 if g.builtin => "Built in".to_string(),
                _ => String::new(),
            }
        }
        Row::Saved(id) => {
            let Some(d) = saved(store, id) else { return String::new() };
            match column {
                0 if store.settings.default_destination == Some(id) => format!("{} (default)", d.name),
                0 => d.name.clone(),
                1 => app::target_summary(store, &d.target),
                2 => d.target.durability().label().to_string(),
                3 => ui::ago(now(), d.last_used),
                4 => one_line(&d.notes),
                _ => String::new(),
            }
        }
    }
}

fn recent_cell(store: &Store, entry: &RecentLaunch, column: i32) -> String {
    match column {
        0 => match store.account(entry.account) {
            Some(a) => a.display_name().to_string(),
            None => "(removed)".to_string(),
        },
        1 => match store.destinations.iter().find(|d| d.target == entry.target) {
            Some(d) => d.name.clone(),
            None => app::target_summary(store, &entry.target),
        },
        2 => entry.target.durability().label().to_string(),
        3 => ui::ago(now(), Some(entry.at)),
        _ => String::new(),
    }
}

fn selected_row(app: &App) -> Option<Row> {
    let v = &app.destinations;
    ui::list_selected(v.list).first().and_then(|i| v.rows.get(*i).copied())
}

fn selected_recent(app: &App) -> Option<RecentLaunch> {
    ui::list_selected(app.destinations.recent).first().and_then(|i| app.store.recent.get(*i).cloned())
}

/// Rebuilds both tables from the store, keeping the selection where the
/// selected entries still exist.
pub fn refresh(app: &mut App) {
    let keep = selected_row(app);
    let keep_recent = selected_recent(app);
    let rows = build_rows(&app.store);
    let pick = keep.and_then(|k| rows.iter().position(|r| *r == k)).unwrap_or(0);
    let pick_recent = keep_recent
        .and_then(|k| app.store.recent.iter().position(|r| r.account == k.account && r.target == k.target));
    let v = &mut app.destinations;
    v.loading = true;
    v.rows = rows;
    ui::list_set_count(v.list, v.rows.len());
    ui::list_set_selected(v.list, None, false);
    if !v.rows.is_empty() {
        ui::list_focus_row(v.list, pick);
    }
    ui::list_set_count(v.recent, app.store.recent.len());
    ui::list_set_selected(v.recent, None, false);
    if let Some(i) = pick_recent {
        ui::list_focus_row(v.recent, i);
    }
    v.loading = false;
    refresh_hint(app);
    refresh_actions(app);
}

/// Selects one row of the saved table.
fn select_row(app: &mut App, row: Row) {
    let Some(index) = app.destinations.rows.iter().position(|r| *r == row) else { return };
    app.destinations.loading = true;
    ui::list_set_selected(app.destinations.list, None, false);
    ui::list_focus_row(app.destinations.list, index);
    app.destinations.loading = false;
    refresh_actions(app);
}

/// Enables the buttons that apply to the current selections.
fn refresh_actions(app: &mut App) {
    let store = &app.store;
    let v = &app.destinations;
    let row = selected_row(app);
    let selected_game = match row {
        Some(Row::Game(place_id)) => game(store, place_id),
        _ => None,
    };
    let selected_saved = match row {
        Some(Row::Saved(id)) => saved(store, id),
        _ => None,
    };
    let editable = selected_saved.is_some() || selected_game.is_some_and(|g| !g.builtin);
    ui::enable(v.launch, row.is_some());
    ui::enable(v.rename, editable);
    ui::enable(v.notes, selected_saved.is_some());
    ui::enable(v.remove, editable);
    ui::enable(v.default, selected_saved.is_some());
    ui::enable(v.pin, selected_game.is_some_and(|g| !g.builtin));
    ui::enable(v.page, row.and_then(|r| row_target(store, r)).and_then(|t| t.place_id()).is_some());
    let is_default = selected_saved.is_some_and(|d| store.settings.default_destination == Some(d.id));
    ui::set_text_if_changed(v.default, if is_default { "Clear default" } else { "Set as default" });
    let pinned = selected_game.is_some_and(|g| g.pinned && !g.builtin);
    ui::set_text_if_changed(v.pin, if pinned { "Unpin" } else { "Pin" });

    let again = selected_recent(app).is_some_and(|r| store.account(r.account).is_some());
    ui::enable(v.again, again);
    ui::enable(v.clear, !store.recent.is_empty());
    for h in [v.launch, v.rename, v.notes, v.remove, v.default, v.pin, v.page, v.again, v.clear] {
        ui::invalidate(h);
    }
}

// ---- adding ----------------------------------------------------------------

/// What a pasted link was recognised as and how durable it is. Never
/// includes a link or share code.
fn recognised(store: &Store, target: &Target) -> String {
    match target {
        Target::Place { place_id } => {
            format!("{} \u{2014} joins the experience; Roblox picks the server.", store.game_name(*place_id))
        }
        Target::PrivateServer { place_id, .. } => format!(
            "Private server link for {}. Saved link: stops working if the owner regenerates it.",
            store.game_name(*place_id)
        ),
        Target::ShareLink { .. } => "Share link. The manager cannot see where it leads; saved link.".to_string(),
        Target::LiveServer { place_id, .. } => format!(
            "One specific server of {}. Live server: it may have closed by the time you launch.",
            store.game_name(*place_id)
        ),
    }
}

/// The name a destination gets when the Name box is empty.
fn default_name(store: &Store, target: &Target) -> String {
    match target {
        Target::Place { place_id } => store.game_name(*place_id),
        Target::PrivateServer { place_id, .. } => format!("{} private server", store.game_name(*place_id)),
        Target::ShareLink { .. } => "Share link".to_string(),
        Target::LiveServer { place_id, .. } => format!("{} server", store.game_name(*place_id)),
    }
}

/// Updates the validation line and the Add button for the link box.
fn refresh_hint(app: &mut App) {
    let v = &app.destinations;
    let (text, valid) = match links::parse_destination(&ui::get_text(v.link)) {
        Ok(target) => {
            let mut text = recognised(&app.store, &target);
            if let Some(d) = app.store.destinations.iter().find(|d| d.target == target) {
                text.push_str(&format!(" Already saved as \"{}\".", d.name));
            }
            (text, true)
        }
        Err(e) => (e.to_string(), false),
    };
    ui::set_text_if_changed(v.hint, &text);
    if unsafe { IsWindowEnabled(v.add).as_bool() } != valid {
        ui::enable(v.add, valid);
        ui::invalidate(v.add);
    }
}

fn add(app: &mut App) {
    let target = match links::parse_destination(&ui::get_text(app.destinations.link)) {
        Ok(t) => t,
        Err(e) => {
            app.set_status(&e.to_string());
            ui::focus(app.destinations.link);
            return;
        }
    };
    if let Some(d) = app.store.destinations.iter().find(|d| d.target == target) {
        let (id, name) = (d.id, d.name.clone());
        select_row(app, Row::Saved(id));
        ui::focus(app.destinations.list);
        app.set_status(&format!("That destination is already saved as \"{name}\"."));
        return;
    }
    let typed = ui::get_text(app.destinations.name).trim().to_string();
    if let Target::Place { place_id } = target {
        if game(&app.store, place_id).is_none() {
            let name = if typed.is_empty() { format!("Place {place_id}") } else { typed.clone() };
            app.store.games.push(Game { place_id, universe_id: None, name, pinned: false, builtin: false });
        }
    }
    let name = if typed.is_empty() { default_name(&app.store, &target) } else { typed };
    let kind = target.durability().label();
    let id = app.store.alloc_id();
    app.store.destinations.push(Destination {
        id,
        name: name.clone(),
        target,
        notes: String::new(),
        created: now(),
        last_used: None,
        import_key: None,
    });
    app.save_now();
    app.destinations.loading = true;
    ui::set_text(app.destinations.link, "");
    ui::set_text(app.destinations.name, "");
    app.destinations.loading = false;
    refresh(app);
    select_row(app, Row::Saved(id));
    ui::focus(app.destinations.list);
    crate::accounts::refresh(app);
    app.set_status(&format!("Added \"{name}\" \u{2014} {kind}."));
}

// ---- actions ---------------------------------------------------------------

/// Saves a structural change and updates everything that lists
/// destinations.
fn commit(app: &mut App) {
    app.save_now();
    refresh(app);
    crate::accounts::refresh(app);
}

fn launch_selected(app: &mut App) {
    let Some(target) = selected_row(app).and_then(|r| row_target(&app.store, r)) else {
        app.set_status("Select a destination to launch to.");
        return;
    };
    let ids = crate::accounts::selected_ids(app);
    if ids.is_empty() {
        app.set_status("No account is selected. Select accounts in the Accounts view (Ctrl+1), then launch from here.");
        return;
    }
    // One account launches as asked, even Main. Several accounts are a
    // batch, which leaves Main out unless Settings says otherwise.
    let batch = if ids.len() > 1 { search::batch_targets(&app.store, &ids) } else { ids.clone() };
    if batch.len() != ids.len() {
        app.set_status("Main was left out of the batch.");
    }
    app.launch(&batch, &target);
    refresh(app);
}

fn rename_selected(app: &mut App) {
    let Some(row) = selected_row(app) else { return };
    if matches!(row, Row::Game(id) if game(&app.store, id).is_none_or(|g| g.builtin)) {
        return;
    }
    let current = row_name(&app.store, row);
    let title = if matches!(row, Row::Game(_)) { "Rename game" } else { "Rename destination" };
    let Some(values) = dialogs::prompt(app.hwnd, title, "", "Rename", &[Field::line("Name", &current, "")], &|v| {
        match v[0].trim().chars().count() {
            0 => Err("Enter a name.".into()),
            1..=80 => Ok(()),
            _ => Err("Names are at most 80 characters.".into()),
        }
    }) else {
        return;
    };
    let name = values[0].trim().to_string();
    match row {
        Row::Game(place_id) => {
            if let Some(g) = app.store.games.iter_mut().find(|g| g.place_id == place_id) {
                g.name = name.clone();
            }
        }
        Row::Saved(id) => {
            if let Some(d) = app.store.destinations.iter_mut().find(|d| d.id == id) {
                d.name = name.clone();
            }
        }
    }
    commit(app);
    app.set_status(&format!("Renamed to \"{name}\"."));
}

fn edit_notes(app: &mut App) {
    let Some(Row::Saved(id)) = selected_row(app) else { return };
    let Some(d) = saved(&app.store, id) else { return };
    let (name, current) = (d.name.clone(), d.notes.replace("\r\n", "\n").replace('\n', "\r\n"));
    let field = Field { label: "Notes", value: &current, cue: "", multiline: true };
    let Some(values) = dialogs::prompt(app.hwnd, &format!("Notes for {name}"), "", "Save", &[field], &|_| Ok(())) else {
        return;
    };
    if let Some(d) = app.store.destinations.iter_mut().find(|d| d.id == id) {
        d.notes = values[0].replace("\r\n", "\n").trim_end().to_string();
    }
    app.save_now();
    ui::invalidate(app.destinations.list);
    app.set_status(&format!("Notes saved for \"{name}\"."));
}

fn remove_selected(app: &mut App) {
    let Some(row) = selected_row(app) else { return };
    let name = row_name(&app.store, row);
    match row {
        Row::Game(place_id) => {
            if game(&app.store, place_id).is_none_or(|g| g.builtin) {
                app.set_status(&format!("{name} is built in and cannot be removed."));
                return;
            }
            let text = format!("Remove {name} from the game list?\n\nSaved destinations for this game are kept.");
            if !ui::confirm(app.hwnd, "Remove game", &text) {
                return;
            }
            app.store.games.retain(|g| g.place_id != place_id);
        }
        Row::Saved(id) => {
            let text = format!(
                "Remove \"{name}\" from the saved destinations?\n\nOnly the saved entry is removed. Nothing changes on Roblox."
            );
            if !ui::confirm(app.hwnd, "Remove destination", &text) {
                return;
            }
            app.store.destinations.retain(|d| d.id != id);
            if app.store.settings.default_destination == Some(id) {
                app.store.settings.default_destination = None;
            }
        }
    }
    commit(app);
    app.set_status(&format!("Removed \"{name}\"."));
}

fn toggle_default(app: &mut App) {
    let Some(Row::Saved(id)) = selected_row(app) else { return };
    let name = row_name(&app.store, Row::Saved(id));
    let settings = &mut app.store.settings;
    settings.default_destination = if settings.default_destination == Some(id) { None } else { Some(id) };
    let text = match settings.default_destination {
        Some(_) => format!("\"{name}\" is now the default destination."),
        None => "No default destination is set.".to_string(),
    };
    commit(app);
    app.set_status(&text);
}

fn toggle_pin(app: &mut App) {
    let Some(Row::Game(place_id)) = selected_row(app) else { return };
    let Some(g) = app.store.games.iter_mut().find(|g| g.place_id == place_id && !g.builtin) else { return };
    g.pinned = !g.pinned;
    let text = format!("{} is {}.", g.name, if g.pinned { "pinned to the top" } else { "no longer pinned" });
    commit(app);
    app.set_status(&text);
}

fn open_page(app: &mut App) {
    let place = selected_row(app).and_then(|r| row_target(&app.store, r)).and_then(|t| t.place_id());
    let Some(place_id) = place else {
        app.set_status("A share link has no game page the manager can open.");
        return;
    };
    match app.browsers.open(None, &links::game_page_url(place_id)) {
        Ok(()) => app.set_status(&format!("Opened the {} page in the default browser.", app.store.game_name(place_id))),
        Err(e) => app.set_status(&format!("The browser could not be started: {e}.")),
    }
}

fn launch_again(app: &mut App) {
    let Some(entry) = selected_recent(app) else { return };
    if app.store.account(entry.account).is_none() {
        app.set_status("That account was removed, so this launch cannot be repeated.");
        return;
    }
    app.launch(&[entry.account], &entry.target);
    refresh(app);
}

fn clear_history(app: &mut App) {
    if app.store.recent.is_empty() {
        return;
    }
    let text = "Clear the recent launch history?\n\nSaved destinations and accounts are not affected.";
    if !ui::confirm(app.hwnd, "Clear history", text) {
        return;
    }
    app.store.recent.clear();
    app.save_now();
    refresh(app);
    app.set_status("Recent launch history cleared.");
}

pub fn command(app: &mut App, id: u16, code: u16, _ctl: HWND) {
    if app.destinations.loading {
        return;
    }
    const EN_CHANGE_CODE: u16 = 0x0300;
    match (id, code) {
        (ID_LINK, EN_CHANGE_CODE) => refresh_hint(app),
        (ID_ADD, 0) => add(app),
        (ID_LAUNCH, 0) => launch_selected(app),
        (ID_RENAME, 0) => rename_selected(app),
        (ID_NOTES, 0) => edit_notes(app),
        (ID_REMOVE, 0) => remove_selected(app),
        (ID_DEFAULT, 0) => toggle_default(app),
        (ID_PIN, 0) => toggle_pin(app),
        (ID_PAGE, 0) => open_page(app),
        (ID_AGAIN, 0) => launch_again(app),
        (ID_CLEAR, 0) => clear_history(app),
        _ => {}
    }
}

// ---- notifications ---------------------------------------------------------

pub fn notify(app: &mut App, hdr: &NMHDR, lparam: LPARAM) -> Option<LRESULT> {
    let recent = match hdr.idFrom as u16 {
        ID_LIST => false,
        ID_RECENT => true,
        _ => return None,
    };
    match hdr.code {
        LVN_GETDISPINFOW => unsafe {
            let info = &mut *(lparam.0 as *mut NMLVDISPINFOW);
            if info.item.mask.contains(LVIF_TEXT) {
                let (index, column) = (info.item.iItem as usize, info.item.iSubItem);
                let text = if recent {
                    app.store.recent.get(index).map(|r| recent_cell(&app.store, r, column))
                } else {
                    app.destinations.rows.get(index).map(|r| cell(&app.store, *r, column))
                };
                ui::fill_dispinfo(&mut info.item, &text.unwrap_or_default());
            }
            Some(LRESULT(0))
        },
        LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
            if !app.destinations.loading {
                refresh_actions(app);
            }
            Some(LRESULT(0))
        }
        NM_DBLCLK => {
            if recent {
                launch_again(app);
            } else {
                launch_selected(app);
            }
            Some(LRESULT(0))
        }
        LVN_KEYDOWN => unsafe {
            let info = &*(lparam.0 as *const NMLVKEYDOWN);
            if !recent && info.wVKey == VK_DELETE.0 {
                remove_selected(app);
            }
            Some(LRESULT(0))
        },
        LVN_GETEMPTYMARKUP if recent => unsafe {
            let info = &mut *(lparam.0 as *mut NMLVEMPTYMARKUP);
            let text = "No launches yet. Accounts you launch appear here so you can send them back.";
            info.dwFlags = EMF_CENTERED;
            for (i, unit) in text.encode_utf16().take(info.szMarkup.len() - 1).enumerate() {
                info.szMarkup[i] = unit;
                info.szMarkup[i + 1] = 0;
            }
            Some(LRESULT(1))
        },
        NM_CUSTOMDRAW => unsafe {
            let cd = &mut *(lparam.0 as *mut NMLVCUSTOMDRAW);
            const PREPAINT: u32 = 0x1;
            const ITEM_PREPAINT: u32 = 0x10001;
            const SUBITEM_PREPAINT: u32 = 0x30001;
            match cd.nmcd.dwDrawStage.0 {
                PREPAINT => Some(LRESULT(CDRF_NOTIFYITEMDRAW as isize)),
                ITEM_PREPAINT => Some(LRESULT(CDRF_NOTIFYSUBITEMDRAW as isize)),
                SUBITEM_PREPAINT => {
                    let index = cd.nmcd.dwItemSpec;
                    let tone = if recent {
                        recent_tone(&app.store, app.store.recent.get(index), cd.iSubItem)
                    } else {
                        let target = app.destinations.rows.get(index).and_then(|r| match r {
                            Row::Saved(id) => saved(&app.store, *id).map(|d| &d.target),
                            Row::Game(_) => None,
                        });
                        match cd.iSubItem {
                            0 => Tone::Text,
                            2 if is_live(target) => Tone::Warn,
                            _ => Tone::Muted,
                        }
                    };
                    cd.clrText = COLORREF(tone_color(&app.palette, tone));
                    Some(LRESULT(CDRF_NEWFONT as isize))
                }
                _ => Some(LRESULT(0)),
            }
        },
        _ => None,
    }
}

fn is_live(target: Option<&Target>) -> bool {
    target.is_some_and(|t| t.durability() == Durability::Live)
}

fn recent_tone(store: &Store, entry: Option<&RecentLaunch>, column: i32) -> Tone {
    match column {
        0 if entry.is_some_and(|r| store.account(r.account).is_some()) => Tone::Text,
        1 => Tone::Text,
        2 if is_live(entry.map(|r| &r.target)) => Tone::Warn,
        _ => Tone::Muted,
    }
}
