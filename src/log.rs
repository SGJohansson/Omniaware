use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: &Path) {
    let _ = PATH.set(dir.join("omniaware.log"));
}

pub fn error(msg: impl std::fmt::Display) {
    let line = format!("[{}] {msg}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    eprint!("{line}");
    if let Some(p) = PATH.get()
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
