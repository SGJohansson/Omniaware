//! Plain-text export: "save as" for one entry, one dump (oldest first) for several.
//! Images live beside the text (attachments), so the text written is exactly the entry text.

use crate::db::Entry;
use crate::text as t;
use chrono::TimeZone;
use std::path::{Path, PathBuf};

/// Extensions that get LF line endings (scripts that run on Unix shells); everything else CRLF.
const LF_EXT: &[&str] = &["sh", "bash", "zsh", "fish"];
/// Windows PowerShell 5.1 reads BOM-less UTF-8 as ANSI and breaks åäö; these get a BOM.
const BOM_EXT: &[&str] = &["ps1", "psm1", "psd1"];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const MAX_STEM: usize = 60;

fn local(ms: i64) -> chrono::DateTime<chrono::Local> {
    chrono::Local.timestamp_millis_opt(ms).single().unwrap_or_else(chrono::Local::now)
}

/// File name for the dialog: the entry name, else its first line, else a timestamp. `.txt` unless
/// the name already ends in an extension (`deploy.ps1`).
pub fn suggest_name(name: &str, body: &str, created: i64) -> String {
    let from_name = sanitize(name.trim());
    let base = if !from_name.is_empty() {
        from_name
    } else {
        let first = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        sanitize(first.trim_start_matches(['#', '>', '-', '*', ' ']))
    };
    let base = if base.is_empty() { format!("omniaware-{}", local(created).format("%Y%m%d-%H%M")) } else { base };
    if has_ext(&base) { base } else { format!("{base}.txt") }
}

/// File name for a dump of `n` entries.
pub fn dump_name(n: usize) -> String {
    format!("omniaware-{n}-entries-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M"))
}

fn has_ext(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Windows-safe file name: no reserved characters or names, no trailing dots/spaces, capped length.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let mut out = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_STEM {
        out = out.chars().take(MAX_STEM).collect();
    }
    let out = out.trim_end_matches(['.', ' ']).to_string();
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) { format!("_{out}") } else { out }
}

fn ext_in(path: &Path, list: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| list.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Bytes as written: UTF-8, CRLF (LF for shell scripts), BOM for PowerShell.
pub fn encode(text: &str, path: &Path) -> Vec<u8> {
    let lf = text.replace("\r\n", "\n");
    let body = if ext_in(path, LF_EXT) { lf } else { lf.replace('\n', "\r\n") };
    let mut out = Vec::with_capacity(body.len() + 3);
    if ext_in(path, BOM_EXT) {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    out.extend_from_slice(body.as_bytes());
    out
}

/// Writes via a temp file + rename, so an existing file is never left half-written.
pub fn write(path: &Path, text: &str) -> std::io::Result<u64> {
    let bytes = encode(text, path);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".omniaware-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, &bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(bytes.len() as u64)
}

/// One entry: its text as is. Several: oldest first, each under a one-line header.
pub fn dump(mut entries: Vec<Entry>) -> String {
    entries.sort_by_key(|e| (e.created, e.id));
    if let [e] = entries.as_slice() {
        return e.body.clone();
    }
    let parts: Vec<String> = entries
        .iter()
        .map(|e| {
            let body = e.body.trim_end();
            if body.is_empty() { header(e) } else { format!("{}\n\n{body}", header(e)) }
        })
        .collect();
    parts.join("\n\n\n") + "\n"
}

/// "── 2026-10-09 14:32 · name · +2 images ──"
fn header(e: &Entry) -> String {
    let mut bits = vec![local(e.created).format("%Y-%m-%d %H:%M").to_string()];
    if let Some(n) = e.name.as_deref().filter(|n| !n.is_empty()) {
        bits.push(n.to_string());
    }
    let k = e.images.len();
    if k > 0 {
        bits.push(format!("+{k} {}", t::plural(k, "image", "images")));
    }
    format!("── {} ──", bits.join(" · "))
}

/// Native "Save as" dialog owned by our window (so it opens above the topmost capture popup).
pub fn ask_path(owner: isize, suggested: &Path) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(t::DLG_SAVE_AS);
    if let Some(dir) = suggested.parent().filter(|p| p.is_dir()) {
        d = d.set_directory(dir);
    }
    if let Some(n) = suggested.file_name() {
        d = d.set_file_name(n.to_string_lossy());
    }
    #[cfg(windows)]
    {
        d = d.set_parent(&owner::Owner(owner));
    }
    #[cfg(not(windows))]
    let _ = owner;
    d.save_file()
}

#[cfg(windows)]
pub mod owner {
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    };
    use std::num::NonZeroIsize;

    /// Our HWND as a dialog parent.
    pub struct Owner(pub isize);

    impl HasWindowHandle for Owner {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let h = NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?;
            // SAFETY: the HWND belongs to our own window, which outlives the modal dialog.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(h))) })
        }
    }

    impl HasDisplayHandle for Owner {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, created: i64, name: Option<&str>, body: &str, images: usize) -> Entry {
        Entry {
            id,
            name: name.map(String::from),
            body: body.into(),
            created,
            starts: None,
            images: (0..images as i64).map(|i| crate::db::Img { id: i, blob: format!("{i}.png") }).collect(),
            export_path: None,
        }
    }

    #[test]
    fn names() {
        assert_eq!(suggest_name("deploy.ps1", "", 0), "deploy.ps1");
        assert_eq!(suggest_name("adress-jobb", "x", 0), "adress-jobb.txt");
        assert_eq!(suggest_name("", "\n  # Inköp: mjölk/ägg?  \nmer", 0), "Inköp mjölk ägg.txt");
        assert_eq!(suggest_name("", "Hello world. This is it", 0), "Hello world. This is it.txt");
        assert_eq!(suggest_name("con", "", 0), "_con.txt");
        assert!(suggest_name("", "   ", 0).starts_with("omniaware-"));
        assert_eq!(suggest_name("", &"a".repeat(200), 0).len(), MAX_STEM + 4);
    }

    #[test]
    fn encoding() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(encode("a\nb\r\nc", &p("x.txt")), b"a\r\nb\r\nc");
        assert_eq!(encode("a\r\nb", &p("x.sh")), b"a\nb");
        assert_eq!(encode("å", &p("x.PS1")), b"\xEF\xBB\xBF\xC3\xA5");
    }

    #[test]
    fn dumps() {
        assert_eq!(dump(vec![entry(1, 5, Some("n"), "exact\n  text\n", 2)]), "exact\n  text\n");
        let d = dump(vec![entry(2, 2_000_000, None, "second\n", 0), entry(1, 1_000_000, Some("first"), "one", 2)]);
        let first = d.find("one").unwrap();
        let second = d.find("second").unwrap();
        assert!(first < second, "oldest first: {d}");
        assert!(d.contains(" · first · +2 images ──\n\none\n\n\n── "));
        assert!(d.ends_with("second\n"));
    }

    #[test]
    fn write_replaces() {
        let p = std::env::temp_dir().join(format!("omni-export-{}.txt", crate::db::now_ms()));
        write(&p, "old\nold").unwrap();
        assert_eq!(write(&p, "ny\n").unwrap(), 4);
        assert_eq!(std::fs::read(&p).unwrap(), b"ny\r\n");
        let _ = std::fs::remove_file(&p);
    }
}
