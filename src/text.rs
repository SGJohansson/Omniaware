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
pub const BTN_EXPORT: &str = "export…";
pub const BTN_OPEN_WITH: &str = "open with…";
pub const BTN_REVEAL: &str = "show in folder";
pub const BTN_REMOVE: &str = "remove";
pub const BTN_CLOSE: &str = "close";
pub const BTN_ADD_COPY: &str = "add copy";
pub const BTN_CANCEL: &str = "cancel";
pub const BTN_RESTORE: &str = "restore";
pub const BTN_DELETE: &str = "delete";
pub const BTN_SURE: &str = "sure?";
pub const BTN_SELECT_ALL: &str = "select all";
pub const BTN_CLEAR: &str = "clear";
pub const BTN_DELETE_FOREVER: &str = "delete forever";
pub const BTN_EMPTY_BIN: &str = "empty bin";

pub fn selected(n: usize) -> String {
    format!("{n} selected")
}

pub fn purge_confirm(n: usize) -> String {
    format!("delete {n} forever? press again")
}

pub fn results(n: usize) -> String {
    format!("{n} {}", plural(n, "result", "results"))
}

pub fn bad_pattern(e: &str) -> String {
    format!("Invalid pattern: {e}")
}

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
pub const SEARCH_HELP: &str = "words match inside words · * ? wildcards · /regex/";
pub const TIP_TO_BIN: &str = "Moves them to the bin";
pub const TIP_COPY_TEXT: &str = "Copy the text";
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

// ---------- send to ----------

pub const SEND_TO: &str = "send to…";
pub const TIP_SEND: &str = "Put the text on a shell's prompt line (not run), or open the path (Ctrl+Enter; Ctrl+Shift+Enter to choose)";
pub const SEND_TITLE: &str = "Send to";
pub const SEND_PWSH: &str = "on the prompt line, not run";
pub const SEND_WSL: &str = "on the prompt line, not run";
pub const SEND_WSL_MANY: &str = "lines shown, Enter runs them";
pub const SEND_CMD: &str = "typed at the prompt, not run";
pub const SEND_CMD_MANY: &str = "lines shown, `call` waits for Enter";
pub const SEND_BROWSER: &str = "opens in the browser";
pub const SEND_NO_PATH: &str = "no path or web address in the text";
pub const SEND_NO_WINDOW: &str = "no previous window";
pub const SEND_WINDOW: &str = "pastes with Ctrl+V";
pub const SEND_WINDOW_MANY: &str = "pastes with Ctrl+V · a terminal may run each line";
pub const SEND_ADMIN: &str = "as administrator";
pub const BTN_SEND: &str = "send";

pub fn send_confirm(target: &str) -> String {
    format!("Send to {target}?")
}

pub fn send_remember(target: &str) -> String {
    format!("don't ask again for {target}")
}

pub fn send_opens(path: &str) -> String {
    let short: String = path.chars().take(40).collect();
    if path.chars().count() > 40 { format!("opens {short}…") } else { format!("opens {short}") }
}

pub fn send_more(n: usize) -> String {
    format!("… {n} more line{}", if n == 1 { "" } else { "s" })
}

pub const TIP_THEME: &str = "Theme: dark → light → system → voidflow (Ctrl+Shift+T)";

pub const FOOT_SAVE: &str = "save";
pub const FOOT_NAME: &str = "name";
pub const FOOT_VERSION: &str = "version";
pub const FOOT_SAVE_AS: &str = "save as";
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

// ---------- export ----------

pub const DLG_SAVE_AS: &str = "Save as plain text";
pub const NOTICE_EXPORTED: &str = "Exported";
pub const TIP_EXPORT: &str = "One plain-text file, oldest first (Ctrl+Shift+S)";
pub const TIP_SAVE_AS: &str = "Save the text to a file (Ctrl+Shift+S)";

pub fn toast_exported(file: &str) -> String {
    format!("✓ Saved as {file}")
}

pub fn exported_detail(entries: usize, size: &str) -> String {
    format!("{entries} {} · {size}", plural(entries, "entry", "entries"))
}

// ---------- notices (above the tray) ----------

pub const NOTICE_SAVED: &str = "Saved";
pub const NOTICE_CAPTURED: &str = "Captured silently";
pub const NOTICE_ALREADY: &str = "Already captured";
pub const NOTICE_FAILED: &str = "Couldn't save";
pub const NOTICE_SEE_LOG: &str = "see omniaware.log";

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

const GLOBAL: &[Row] = &[
    (&["Ctrl", "C", "C"], "silent capture"),
    (&["Ctrl", "Alt", "O"], "note → expand → close"),
    (&["Ctrl", "Shift", "T"], "theme: dark → light → system → voidflow"),
];
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
            (&["Ctrl", "Shift", "S"], "save as file"),
            (&["F2"], "name"),
            (&["Ctrl", "V"], "paste image"),
            (&["Ctrl", "Click"], "open link"),
            (&["Ctrl", "Alt", "O"], "expand"),
            (&["Ctrl", "Enter"], "send to suggested shell"),
            (&["Ctrl", "Shift", "Enter"], "send to…"),
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
            (&["Ctrl", "Shift", "S"], "save as file"),
            (&["F2"], "name"),
            (&["Ctrl", "E"], "preview"),
            (&["Ctrl", "Enter"], "send to suggested shell"),
            (&["Ctrl", "Shift", "Enter"], "send to…"),
            (&["Ctrl", "V"], "paste image"),
            (&["Ctrl", "Click"], "open link"),
            (&["Esc"], "back"),
        ],
    ),
    (
        "Lists",
        &[
            (&["Ctrl", "Click"], "select / deselect"),
            (&["Shift", "Click"], "select a range"),
            (&["Ctrl", "Shift", "Click"], "add a range"),
            (&["Ctrl", "A"], "select all in the list"),
            (&["Ctrl", "Shift", "S"], "export as one file"),
            (&["Delete"], "move to bin"),
            (&["Esc"], "clear selection"),
        ],
    ),
    (SEARCH, &[(&["↑", "↓"], "select"), (&["Enter"], "open"), (&["Shift", "Enter"], "paste")]),
    ("Images", IMAGES),
    ("Global", GLOBAL),
];
