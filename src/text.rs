//! Every user-facing string in one place (British English).
//! Log and error-file messages stay next to the code that writes them.

// ---------- dates ----------

pub const WD_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const WD_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
/// Calendar column heads, Monday first.
pub const WD_LETTER: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

// ---------- app & tray ----------

pub const TRAY_CAPTURE: &str = "Quick note";
pub const TRAY_MAIN: &str = "Open Omniaware";
pub const TRAY_QUIT: &str = "Quit";

// ---------- sections ----------

pub const QUICK_NOTE: &str = "Quick note";
pub const JOURNAL: &str = "Journal";
pub const TIMELINE: &str = "Timeline";
pub const NAMED: &str = "Named";
pub const BIN: &str = "Bin";
pub const SEARCH: &str = "Search";
pub const ENTRY: &str = "Entry";
pub const NAV_TIMELINE: &str = "◔  Timeline";
pub const NAV_NAMED: &str = "#  Named";
pub const NAV_BIN: &str = "⌫  Bin";

// ---------- buttons (lower case by design) ----------

pub const BTN_NEW: &str = "+ new";
pub const BTN_BACK: &str = "← back";
pub const BTN_DISCARD: &str = "discard";
pub const BTN_PREVIEW: &str = "preview";
pub const BTN_EDIT: &str = "edit";
pub const BTN_TODAY: &str = "today";
pub const BTN_COPY: &str = "copy";
pub const BTN_SAVE_AS: &str = "save as…";
pub const BTN_OPEN_WITH: &str = "open with…";
pub const BTN_REVEAL: &str = "show in folder";
pub const BTN_REMOVE: &str = "remove";
pub const BTN_CLOSE: &str = "close";
pub const BTN_ADD_COPY: &str = "add copy";
pub const BTN_CANCEL: &str = "cancel";
pub const BTN_RESTORE: &str = "restore";
pub const BTN_DELETE: &str = "delete";
pub const BTN_SURE: &str = "sure?";

pub fn btn_remove_n(n: usize) -> String {
    if n > 1 { format!("remove {n} images") } else { BTN_REMOVE.to_string() }
}

// ---------- tooltips & hints ----------

pub const TIP_CLOSE: &str = "Close (Esc)";
pub const TIP_COLLAPSE: &str = "Collapse (F1)";
pub const TIP_SHORTCUTS: &str = "Shortcuts (F1)";
pub const TIP_F1: &str = "Click or hold F1";
pub const TIP_TILE: &str = "Double-click: enlarge · Ctrl+click: select several · Right-click: menu";
pub const SHORTCUTS: &str = "Shortcuts";
pub const SHORTCUTS_TAB: &str = "?  shortcuts · F1";
pub const SEARCH_HINT: &str = "Search everything…";
pub const NAME_HINT_MAIN: &str = "name (optional, F2)";
pub const NAME_HINT: &str = "e.g. address-work";
pub const NAME_LABEL: &str = "Name";
pub const NAME_BACK: &str = "back to text";
pub const NAME_CLOSE: &str = "save & close";
pub const EDITOR_HINT: &str = "Type or paste…";
pub const EDITOR_HINT_MAIN: &str = "Write… Markdown works, Ctrl+E previews.";
pub const CAPTION_HINT: &str = "Add a caption…";

pub fn link_tip(url: &str) -> String {
    format!("ctrl+click to open {url}")
}

// ---------- capture footer ----------

pub const FOOT_SAVE: &str = "save";
pub const FOOT_NAME: &str = "name";
pub const FOOT_VERSION: &str = "version";
pub const FOOT_EXPAND: &str = "expand";
pub const FOOT_ALL: &str = "all shortcuts";

pub fn dest_named(name: Option<&str>) -> String {
    match name {
        Some(n) => format!("→ {NAMED} · {n}"),
        None => format!("→ {NAMED}"),
    }
}

pub fn dest_journal(day: &str) -> String {
    format!("→ {JOURNAL} · {day}")
}

// ---------- save status ----------

pub const ST_WAITING: &str = "Changes waiting to be saved";
pub const ST_AUTOSAVE: &str = "Saves automatically";
pub const ST_ON_DISK: &str = "Everything is on disk. Ctrl+S saves a version.";
pub const ST_NOTHING: &str = "Nothing to save yet";
pub const ST_FAILED: &str = "save failed";

pub fn st_on_disk_at(t: &str) -> String {
    format!("Everything is on disk (last at {t}). Ctrl+S saves a version.")
}

pub fn toast_saved(n: i64) -> String {
    format!("✓ Saved · version {n}")
}

pub const LEGEND_TITLE: &str = "Status dot";
pub const LEGEND_EMPTY: &str = "empty – nothing to save";
pub const LEGEND_SAVED: &str = "saved to disk";
pub const LEGEND_WAITING: &str = "waiting to save";
pub const LEGEND_ERROR: &str = "error – see the log";
pub const LEGEND_RING: &str = "A ring = something was just saved.";

// ---------- naming ----------

pub const NAME_EMPTY: &str = "Empty entry, nothing to name.";

pub fn name_taken(name: &str) -> String {
    format!("“{name}” already exists. Press Enter again to move the name here; the old entry keeps its text, unnamed.")
}

// ---------- images ----------

pub const SIDE_SIZE: &str = "size";
pub const SIDE_DIMS: &str = "dims";

pub fn copy_of(n: usize, m: usize) -> String {
    format!("#{n} · copy of #{m}")
}

pub fn dup_title(n: usize) -> String {
    format!("This image is identical to #{n}")
}

pub const DUP_NOTE: &str = "Same file, no extra space";

// ---------- timeline ----------

pub const ROW_EMPTY: &str = "(empty)";
pub const ROW_IMAGE: &str = "image";
pub const ROW_IMAGES: &str = "images";

pub fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

pub fn day_meta(entries: usize, events: usize) -> String {
    let mut s = format!("{entries} {}", plural(entries, "entry", "entries"));
    if events > 0 {
        s.push_str(&format!(" · {events} {}", plural(events, "event", "events")));
    }
    s
}

pub fn empty_day(caps: &str) -> String {
    format!("Nothing here yet. Ctrl+C+C saves the clipboard instantly, {caps} opens a quick note, Ctrl+N starts a new entry.")
}

pub const EMPTY_NAMED: &str = "No named entries yet. Press F2 in a note to give it a name.";
pub const EMPTY_BIN: &str = "The bin is empty.";
pub const NO_MATCHES: &str = "No matches.";

// ---------- shortcut lists ----------

/// Keycaps + description.
pub type Row = (&'static [&'static str], &'static str);
pub type Group = (&'static str, &'static [Row]);

const GLOBAL: &[Row] = &[(&["Ctrl", "C", "C"], "silent capture"), (&["Ctrl", "Alt", "O"], "note → expand → close")];
const IMAGES: &[Row] = &[
    (&["Click"], "select"),
    (&["Ctrl", "Click"], "select several"),
    (&["Double-click"], "enlarge"),
    (&["Right-click"], "menu"),
    (&["Delete"], "remove selected"),
];

pub const CAPTURE_KEYS: &[Group] = &[
    (
        QUICK_NOTE,
        &[
            (&["Esc"], "save and close"),
            (&["Ctrl", "S"], "save a version"),
            (&["F2"], "name"),
            (&["Ctrl", "V"], "paste image"),
            (&["Ctrl", "Click"], "open link"),
            (&["Ctrl", "Alt", "O"], "expand"),
            (&["Shift", "Esc"], "discard"),
        ],
    ),
    ("Images", IMAGES),
    ("Global", GLOBAL),
];

pub const MAIN_KEYS: &[Group] = &[
    (TIMELINE, &[(&["←", "→"], "day"), (&["T"], "today"), (&["Ctrl", "N"], "new entry"), (&["Ctrl", "K"], "search")]),
    (
        ENTRY,
        &[
            (&["Ctrl", "S"], "save a version"),
            (&["F2"], "name"),
            (&["Ctrl", "E"], "preview"),
            (&["Ctrl", "V"], "paste image"),
            (&["Ctrl", "Click"], "open link"),
            (&["Esc"], "back"),
        ],
    ),
    (SEARCH, &[(&["↑", "↓"], "select"), (&["Enter"], "open"), (&["Shift", "Enter"], "paste")]),
    ("Images", IMAGES),
    ("Global", GLOBAL),
];
