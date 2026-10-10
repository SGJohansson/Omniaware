//! Send a note's text to PowerShell, cmd, WSL, Explorer or the previous window.
//!
//! Shells open in their own console window with the text on the prompt line, never run: you
//! read it there and press Enter yourself.
//! - PowerShell inserts it through PSReadLine once the prompt is idle (multi-line stays one
//!   editable block).
//! - WSL (the default distribution) shows it in an editable `read -e` prompt line; several lines
//!   are printed and the prompt line sources them instead.
//! - cmd gets it typed into its console input; several lines become `call "<file>"`.
//!
//! "As administrator" starts a second Omniaware elevated (`omniaware --send <target> <file>`),
//! which opens the shell and exits, so one UAC prompt covers everything.

#![cfg_attr(not(windows), allow(dead_code))]

use crate::detect::Kind;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Pwsh,
    Cmd,
    Wsl,
    Explorer,
    /// Paste (Ctrl+V) into the window that was in front before Omniaware.
    Window,
}

impl Target {
    pub const ALL: [Target; 5] = [Target::Pwsh, Target::Cmd, Target::Wsl, Target::Explorer, Target::Window];

    /// Name in config.toml and on the `--send` command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pwsh => "pwsh",
            Self::Cmd => "cmd",
            Self::Wsl => "wsl",
            Self::Explorer => "explorer",
            Self::Window => "window",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pwsh => "PowerShell",
            Self::Cmd => "cmd",
            Self::Wsl => "WSL",
            Self::Explorer => "Explorer",
            Self::Window => "previous window",
        }
    }

    /// The suggestion for detected text.
    pub fn for_kind(k: Option<Kind>) -> Option<Self> {
        Some(match k? {
            Kind::Pwsh => Self::Pwsh,
            Kind::Bash => Self::Wsl,
            Kind::Cmd => Self::Cmd,
            Kind::WinPath | Kind::UnixPath | Kind::Url => Self::Explorer,
        })
    }

    pub fn can_elevate(self) -> bool {
        matches!(self, Self::Pwsh | Self::Cmd | Self::Wsl)
    }

    /// File extension for the text handed to this shell.
    fn ext(self) -> &'static str {
        match self {
            Self::Pwsh => "ps1",
            Self::Cmd => "cmd",
            Self::Wsl => "sh",
            Self::Explorer | Self::Window => "txt",
        }
    }
}

/// Where the hand-over files live (cleared of old ones on start).
pub fn spool(data: &Path) -> PathBuf {
    data.join("send")
}

/// Removes hand-over files older than a day.
pub fn clean_spool(data: &Path) {
    let Ok(rd) = std::fs::read_dir(spool(data)) else { return };
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Writes `text` for `target` into the spool and returns the file.
pub fn stage(data: &Path, target: Target, text: &str) -> Result<PathBuf, String> {
    let dir = spool(data);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("oa-{}.{}", crate::db::now_ms(), target.ext()));
    crate::export::write(&path, text.trim_end_matches(['\r', '\n'])).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

// ---------- command lines and scripts (pure, tested) ----------

/// `-EncodedCommand` payload: base64 of UTF-16LE. Avoids every quoting problem.
pub fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Puts the file's text on the PSReadLine prompt line when the shell first goes idle.
pub fn pwsh_script(file: &Path) -> String {
    format!(
        "$global:__oaFile = {f}\n\
         $null = Register-EngineEvent -SourceIdentifier PowerShell.OnIdle -MaxTriggerCount 1 -Action {{\n\
         \x20 try {{\n\
         \x20   $t = [IO.File]::ReadAllText($global:__oaFile).Replace(\"`r`n\", \"`n\").TrimEnd(\"`n\")\n\
         \x20   [Microsoft.PowerShell.PSConsoleReadLine]::Insert($t)\n\
         \x20 }} catch {{\n\
         \x20   Write-Host \"Omniaware could not put the text on the prompt line ($_). It is in $global:__oaFile\"\n\
         \x20 }}\n\
         }}\n",
        f = ps_quote(&file.to_string_lossy())
    )
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `C:\Users\x` → `/mnt/c/Users/x` (WSL's default automount root).
pub fn wsl_path(win: &str) -> Option<String> {
    let b = win.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return None;
    }
    let rest = win[2..].replace('\\', "/");
    let rest = rest.trim_start_matches('/');
    Some(format!("/mnt/{}/{rest}", (b[0] as char).to_ascii_lowercase()).trim_end_matches('/').to_string())
}

/// `/mnt/c/Users/x` → `C:\Users\x`.
pub fn win_path(unix: &str) -> Option<String> {
    let rest = unix.strip_prefix("/mnt/")?;
    let mut it = rest.splitn(2, '/');
    let drive = it.next()?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let tail = it.next().unwrap_or("").replace('/', "\\");
    Some(format!("{}:\\{tail}", drive.to_ascii_uppercase()))
}

/// bash --rcfile: the normal ~/.bashrc, then the text on an editable prompt line (one line) or
/// printed with a prompt line that sources it (several lines). Never runs anything by itself.
pub fn bash_rc(text_file: &str, text: &str) -> String {
    let f = sh_quote(text_file);
    let one_line = !text.trim_end_matches(['\r', '\n']).contains('\n');
    let fill = if one_line {
        "__oa_fill=$(cat -- \"$__oa_f\")".to_string()
    } else {
        let n = text.trim_end_matches(['\r', '\n']).lines().count();
        format!(
            "printf '\\033[2m# Omniaware: {n} lines, Enter runs them in this shell\\033[0m\\n'\n\
             cat -- \"$__oa_f\"; printf '\\n'\n\
             __oa_fill=\". $(printf '%q' \"$__oa_f\")\""
        )
    };
    format!(
        "# Omniaware hand-over: load the usual startup file, then offer the text unrun.\n\
         [ -f ~/.bashrc ] && . ~/.bashrc\n\
         __oa_f={f}\n\
         {fill}\n\
         if IFS= read -r -e -i \"$__oa_fill\" -p \"${{PS1@P}}\" __oa_line; then\n\
         \x20 history -s -- \"$__oa_line\"\n\
         \x20 eval -- \"$__oa_line\"\n\
         fi\n\
         unset __oa_f __oa_fill __oa_line\n"
    )
}

/// What is typed into cmd: the text itself (one line, tabs as spaces: Tab completes in cmd), or
/// `call "<file>"` for several lines or very long text.
pub fn cmd_line(text: &str, file: &Path) -> String {
    let t = text.trim_end_matches(['\r', '\n']);
    if t.contains('\n') || t.len() > 8000 {
        format!("call \"{}\"", file.display())
    } else {
        t.replace('\t', "    ")
    }
}

/// A Windows path for Explorer from what was written in the note (quotes stripped).
pub fn explorer_path(text: &str) -> Option<String> {
    let t = text.trim().trim_matches('"');
    match crate::detect::detect(t)? {
        Kind::WinPath => Some(t.to_string()),
        Kind::UnixPath => win_path(t),
        _ => None,
    }
}

// ---------- launching ----------

/// Settings a launch needs (from config.toml's [send]).
#[derive(Clone, Debug)]
pub struct Opts {
    /// Start folder; empty = the home folder.
    pub cwd: String,
    /// PowerShell executable; empty = pwsh.exe if installed, else Windows PowerShell.
    pub pwsh: String,
}

impl Opts {
    pub fn from(cfg: &crate::config::Config) -> Self {
        Self { cwd: cfg.send.cwd.clone(), pwsh: cfg.send.pwsh.clone() }
    }

    fn cwd_win(&self) -> PathBuf {
        if !self.cwd.trim().is_empty() {
            return PathBuf::from(self.cwd.trim());
        }
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| ".".into())
    }

    fn pwsh_exe(&self) -> String {
        if !self.pwsh.trim().is_empty() {
            return self.pwsh.trim().to_string();
        }
        let found = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("pwsh.exe").is_file()));
        if found { "pwsh.exe".into() } else { "powershell.exe".into() }
    }
}

/// Opens `target` with the staged text in `file`. Explorer is handled by `explore`.
pub fn launch(target: Target, file: &Path, text: &str, opts: &Opts) -> Result<(), String> {
    imp::launch(target, file, text, opts)
}

/// Same as `launch`, from an elevated copy of Omniaware (one UAC prompt).
pub fn launch_elevated(target: Target, file: &Path, owner: isize) -> Result<(), String> {
    imp::launch_elevated(target, file, owner)
}

/// `omniaware --send <target> <file>`: the elevated half of "as administrator".
pub fn run_helper(args: &[String]) -> i32 {
    let (Some(t), Some(f)) = (args.first().and_then(|t| Target::parse(t)), args.get(1)) else {
        return 2;
    };
    let dir = crate::config::data_dir();
    crate::log::init(&dir);
    let cfg = crate::config::load(&dir);
    let file = PathBuf::from(f);
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    match launch(t, &file, &text, &Opts::from(&cfg)) {
        Ok(()) => 0,
        Err(e) => {
            crate::log::error(format!("send ({} as administrator): {e}", t.label()));
            1
        }
    }
}

/// Opens the path or address in the text: Explorer (a file is selected in its folder), the
/// browser for web addresses, and Linux paths outside /mnt through WSL's own explorer.exe.
pub fn explore(text: &str) -> Result<(), String> {
    let t = text.trim().trim_matches('"');
    match crate::detect::detect(t) {
        Some(Kind::Url) => {
            crate::theme::open_url(t);
            Ok(())
        }
        Some(Kind::WinPath) | Some(Kind::UnixPath) => match explorer_path(t) {
            Some(p) => imp::explorer(&p),
            None => imp::explorer_wsl(t),
        },
        _ => Err("no path or web address to open".into()),
    }
}

#[cfg(windows)]
mod imp {
    use super::{Opts, Target, bash_rc, cmd_line, encode_command, pwsh_script, wsl_path};
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HWND};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Console::{
        AttachConsole, FreeConsole, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, WriteConsoleInputW,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};

    /// A console is per process: one attach at a time.
    static CONSOLE: Mutex<()> = Mutex::new(());

    /// conhost.exe gives a classic console window even where Windows Terminal is the default.
    fn conhost() -> Command {
        Command::new("conhost.exe")
    }

    pub fn launch(target: Target, file: &Path, text: &str, opts: &Opts) -> Result<(), String> {
        let cwd = opts.cwd_win();
        match target {
            Target::Pwsh => {
                let script = pwsh_script(file);
                conhost()
                    .args([opts.pwsh_exe().as_str(), "-NoLogo", "-NoExit", "-EncodedCommand", &encode_command(&script)])
                    .current_dir(&cwd)
                    .spawn()
                    .map(drop)
                    .map_err(|e| format!("PowerShell: {e}"))
            }
            Target::Wsl => {
                let lin = |p: &Path| wsl_path(&p.to_string_lossy()).ok_or_else(|| format!("not on a drive WSL can see: {}", p.display()));
                let text_lin = lin(file)?;
                let rc = file.with_extension("rc.sh");
                crate::export::write(&rc, &bash_rc(&text_lin, text)).map_err(|e| format!("{}: {e}", rc.display()))?;
                let rc_lin = lin(&rc)?;
                let cd = if opts.cwd.trim().is_empty() { "~".to_string() } else { opts.cwd.trim().to_string() };
                conhost()
                    .args(["wsl.exe", "--cd", &cd, "-e", "bash", "--rcfile", &rc_lin])
                    .current_dir(&cwd)
                    .spawn()
                    .map(drop)
                    .map_err(|e| format!("WSL: {e}"))
            }
            Target::Cmd => {
                let line = cmd_line(text, file);
                // UTF-8 code page so åäö survive; several lines are shown before `call`.
                let mut cl = String::from("cmd.exe /k chcp 65001 >nul");
                if line.starts_with("call \"") {
                    cl.push_str(&format!(" & type \"{}\"", file.display()));
                }
                let child = conhost().raw_arg(&cl).current_dir(&cwd).spawn().map_err(|e| format!("cmd: {e}"))?;
                type_into_child(child.id(), "cmd.exe", &line)
            }
            Target::Explorer | Target::Window => Err("not a shell".into()),
        }
    }

    pub fn launch_elevated(target: Target, file: &Path, owner: isize) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let params = format!("--send {} \"{}\"", target.name(), file.display());
        let r = unsafe {
            ShellExecuteW(
                (owner != 0).then_some(HWND(owner as *mut _)),
                w!("runas"),
                &HSTRING::from(exe.as_os_str()),
                &HSTRING::from(params.as_str()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute reports success as a value above 32; a declined UAC prompt is below.
        if r.0 as isize > 32 { Ok(()) } else { Err("not started (administrator prompt declined?)".into()) }
    }

    pub fn explorer(path: &str) -> Result<(), String> {
        let p = Path::new(path);
        let r = if p.is_file() {
            Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn()
        } else {
            Command::new("explorer.exe").raw_arg(format!("\"{path}\"")).spawn()
        };
        r.map(drop).map_err(|e| format!("Explorer: {e}"))
    }

    /// Linux paths outside /mnt: WSL's explorer.exe opens them as \\wsl.localhost\… itself.
    pub fn explorer_wsl(path: &str) -> Result<(), String> {
        let script = r#"p=$1; case $p in "~"*) p="$HOME${p#\~}";; esac; cd -- "$p" 2>/dev/null || cd -- "$(dirname -- "$p")" || exit 1; explorer.exe ."#;
        Command::new("wsl.exe")
            .args(["-e", "sh", "-c", script, "omniaware", path])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(drop)
            .map_err(|e| format!("WSL: {e}"))
    }

    /// Process id of `exe`, started by `parent` (conhost starts the shell as its child).
    fn child_of(parent: u32, exe: &str) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                if e.th32ParentProcessID == parent && name.eq_ignore_ascii_case(exe) {
                    found = Some(e.th32ProcessID);
                    break;
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    fn key(c: u16, down: bool) -> INPUT_RECORD {
        INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: down.into(),
                    wRepeatCount: 1,
                    wVirtualKeyCode: 0,
                    wVirtualScanCode: 0,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: c },
                    dwControlKeyState: 0,
                },
            },
        }
    }

    /// Types `line` (no Enter) into the console of `exe`, a child of conhost process `host`.
    fn type_into_child(host: u32, exe: &str, line: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let pid = loop {
            if let Some(p) = child_of(host, exe) {
                break p;
            }
            if Instant::now() > deadline {
                return Err(format!("{exe} did not start"));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let recs: Vec<INPUT_RECORD> = line.encode_utf16().flat_map(|c| [key(c, true), key(c, false)]).collect();
        let _guard = CONSOLE.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let _ = FreeConsole();
            let mut attached = AttachConsole(pid);
            while attached.is_err() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
                attached = AttachConsole(pid);
            }
            attached.map_err(|e| format!("attach to {exe}: {e}"))?;
            let res = CreateFileW(
                w!("CONIN$"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .map_err(|e| format!("console input: {e}"))
            .and_then(|h| {
                let mut n = 0u32;
                let r = WriteConsoleInputW(h, &recs, &mut n).map_err(|e| format!("type into {exe}: {e}"));
                let _ = CloseHandle(h);
                r
            });
            let _ = FreeConsole();
            res
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Opts, Target};
    use std::path::Path;

    pub fn launch(_t: Target, _f: &Path, _text: &str, _o: &Opts) -> Result<(), String> {
        Err("sending to a shell works on Windows only".into())
    }
    pub fn launch_elevated(_t: Target, _f: &Path, _owner: isize) -> Result<(), String> {
        Err("Windows only".into())
    }
    pub fn explorer(_p: &str) -> Result<(), String> {
        Err("Windows only".into())
    }
    pub fn explorer_wsl(_p: &str) -> Result<(), String> {
        Err("Windows only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // `dir` as UTF-16LE, as PowerShell's -EncodedCommand expects.
        assert_eq!(encode_command("dir"), "ZABpAHIA");
    }

    #[test]
    fn paths_between_windows_and_wsl() {
        assert_eq!(wsl_path(r"C:\Users\lunal\AppData").as_deref(), Some("/mnt/c/Users/lunal/AppData"));
        assert_eq!(wsl_path(r"K:\").as_deref(), Some("/mnt/k"));
        assert_eq!(wsl_path(r"\\nas\share"), None);
        assert_eq!(win_path("/mnt/k/VFSH/omfile").as_deref(), Some(r"K:\VFSH\omfile"));
        assert_eq!(win_path("/mnt/c").as_deref(), Some(r"C:\"));
        assert_eq!(win_path("/home/slimbo"), None);
    }

    #[test]
    fn cmd_gets_one_line_or_a_call() {
        let f = Path::new(r"C:\x\oa-1.cmd");
        assert_eq!(cmd_line("dir /b\t*.txt\r\n", f), "dir /b    *.txt");
        assert_eq!(cmd_line("echo a\necho b", f), r#"call "C:\x\oa-1.cmd""#);
    }

    #[test]
    fn pwsh_script_quotes_the_path() {
        let s = pwsh_script(Path::new(r"C:\Users\O'Neil\oa-1.ps1"));
        assert!(s.contains(r"'C:\Users\O''Neil\oa-1.ps1'"));
        assert!(s.contains("PSConsoleReadLine]::Insert"));
        assert!(s.contains("-MaxTriggerCount 1"));
    }

    #[test]
    fn bash_rc_never_runs_by_itself() {
        let one = bash_rc("/mnt/c/x/it's.sh", "ls -la");
        assert!(one.contains(r"__oa_f='/mnt/c/x/it'\''s.sh'"));
        assert!(one.contains("read -r -e -i"));
        let many = bash_rc("/mnt/c/x/a.sh", "echo 1\necho 2\n");
        assert!(many.contains("2 lines"));
        assert!(many.contains("__oa_fill=\". "));
    }

    #[test]
    fn suggestions() {
        assert_eq!(Target::for_kind(Some(Kind::Bash)), Some(Target::Wsl));
        assert_eq!(Target::for_kind(Some(Kind::Url)), Some(Target::Explorer));
        assert_eq!(Target::for_kind(None), None);
        assert_eq!(Target::parse("cmd"), Some(Target::Cmd));
        assert_eq!(explorer_path("\"/mnt/k/VFSH\"").as_deref(), Some(r"K:\VFSH"));
    }
}
