//! Benchmark mode, for `oa-probe.exe` (see `src/bin/oa-probe.rs`).
//!
//! With `OMNIAWARE_BENCH=1` (and `OMNIAWARE_DATA` pointing next to the probe) Omniaware runs as
//! a guest beside any real copy on the same machine:
//! - its own single-instance lock, so a real Omniaware keeps running;
//! - no global hotkeys, so the real copy keeps its keys;
//! - screenshots go to `<data>\shots` and never touch the clipboard;
//! - the probe drives it through three named events instead of key presses.
//!
//! Everything else (window, tray, clipboard listener, database) runs as usual, because that is
//! what is being measured.

use std::sync::OnceLock;

/// Names shared with `oa-probe` (kept in sync by hand: the probe is a separate binary).
pub const EVENTS: [&str; 3] = ["Local\\OmniawareBench.Capture", "Local\\OmniawareBench.ShotFull", "Local\\OmniawareBench.Quit"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Same as the capture hotkey: popup → main window → hidden.
    Capture,
    /// Same as Ctrl+Alt+Shift+S.
    ShotFull,
    Quit,
}

pub fn active() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("OMNIAWARE_BENCH").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Waits for the probe's events on a thread of its own (blocked, no polling) and calls `f`.
#[cfg(windows)]
pub fn listen(f: impl Fn(Command) + Send + 'static) {
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForMultipleObjects};
    use windows::core::HSTRING;
    std::thread::spawn(move || unsafe {
        let mut handles: Vec<HANDLE> = Vec::new();
        for name in EVENTS {
            match CreateEventW(None, false, false, &HSTRING::from(name)) {
                Ok(h) => handles.push(h),
                Err(e) => return crate::log::error(format!("bench event {name}: {e}")),
            }
        }
        loop {
            let r = WaitForMultipleObjects(&handles, false, INFINITE);
            let i = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
            let cmd = match i {
                0 => Command::Capture,
                1 => Command::ShotFull,
                2 => Command::Quit,
                _ => return crate::log::error(format!("bench wait: {:#x}", r.0)),
            };
            f(cmd);
            if cmd == Command::Quit {
                return;
            }
        }
    });
}

#[cfg(not(windows))]
pub fn listen(_f: impl Fn(Command) + Send + 'static) {}
