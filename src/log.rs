use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: &Path) {
    let _ = PATH.set(dir.join("omniaware.log"));
}

pub fn error(msg: impl std::fmt::Display) {
    write(format_args!("{msg}"));
}

/// Not an error, but worth finding later (slow window opens, dropped hotkey presses).
pub fn note(msg: impl std::fmt::Display) {
    write(format_args!("note: {msg}"));
}

fn write(msg: std::fmt::Arguments) {
    let line = format!("[{}] {msg}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    eprint!("{line}");
    if let Some(p) = PATH.get()
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

// ---------- crashes ----------

/// Written next to the log when Omniaware dies; the next start shows it once and removes it.
const CRASH_FILE: &str = "crashed.txt";

fn crash_path() -> Option<PathBuf> {
    PATH.get().and_then(|p| p.parent()).map(|d| d.join(CRASH_FILE))
}

fn record_crash(msg: &str) {
    write(format_args!("CRASH: {msg}"));
    if let Some(p) = crash_path() {
        let _ = std::fs::write(p, msg);
    }
}

/// Panics and native crashes are written to the log instead of vanishing silently (a GUI
/// build has no console). Call after `init`.
pub fn install_crash_handlers() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().map(|n| format!(" (thread {n})")).unwrap_or_default();
        record_crash(&format!("panic: {msg}{at}{thread}"));
    }));
    #[cfg(windows)]
    native::install();
}

/// The message of a crash in the previous run, if there was one (removed once read).
pub fn take_crash() -> Option<String> {
    let p = crash_path()?;
    let msg = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    Some(msg.trim().to_string())
}

#[cfg(windows)]
mod native {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    use windows::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
        GetModuleHandleExW,
    };
    use windows::core::PCWSTR;

    /// Access violations and the like: code, address and the module it happened in.
    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            if let Some(rec) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) {
                let addr = rec.ExceptionAddress as usize;
                let mut module = HMODULE::default();
                let place = if GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                    PCWSTR(addr as *const u16),
                    &mut module,
                )
                .is_ok()
                {
                    let mut buf = [0u16; 260];
                    let n = GetModuleFileNameW(Some(module), &mut buf) as usize;
                    let path = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
                    let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                    format!("{name}+{:#x}", addr - module.0 as usize)
                } else {
                    format!("{addr:#x}")
                };
                super::record_crash(&format!("exception {:#010x} in {place}", rec.ExceptionCode.0 as u32));
            }
        }
        0 // EXCEPTION_CONTINUE_SEARCH: Windows still records it as usual
    }

    pub fn install() {
        unsafe {
            SetUnhandledExceptionFilter(Some(filter));
        }
    }
}
