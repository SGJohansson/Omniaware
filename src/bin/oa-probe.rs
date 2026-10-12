//! oa-probe: measures what Omniaware costs on this machine, unattended.
//!
//! Put `oa-probe.exe` and `omniaware.exe` in one folder and run the probe. It writes
//! everything to `bench\<COMPUTERNAME>\` in that folder:
//! - `machine.json`: CPU, clocks, memory, GPU and driver, displays, Windows, power plan;
//! - `samples-<start>.csv`: the Omniaware process every 10 s (1 s during scenarios);
//! - `summary.txt`: averages and peaks per renderer and phase, rewritten every hour;
//! - `oa-data\`: the benchmark copy's own database, log and screenshots.
//!
//! The benchmark copy (`OMNIAWARE_BENCH=1`) runs beside a real Omniaware without touching it:
//! no global hotkeys, no clipboard writes, its own lock and data. Renderers alternate (wgpu,
//! glow) every two hours. Scenarios (quick note → main window → close, a full-screen shot) run
//! once an hour, and only after ten minutes without keyboard or mouse input.
//! Closing the probe closes its Omniaware too (job object).
//!
//! Options: --hours N (72) · --interval S (10) · --switch H (2) · --idle M (10) · --no-scenarios
//! · --hidden (no console window; a tray icon stops it, see `start-hidden.cmd`)

#[cfg(not(windows))]
fn main() {
    eprintln!("oa-probe runs on Windows only.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    std::process::exit(probe::run());
}

#[cfg(windows)]
mod probe {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, LPARAM, LocalFree, RECT};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIDevice, IDXGIFactory1};
    use windows::Win32::Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
        MONITORINFOEXW,
    };
    use windows::Win32::System::Console::SetConsoleCtrlHandler;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::Win32::System::Performance::{
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW,
        PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    };
    use windows::Win32::System::Power::{CallNtPowerInformation, PowerGetActiveScheme, PowerReadFriendlyName, ProcessorInformation};
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, RelationProcessorCore,
        RelationProcessorPackage, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    use windows::Win32::System::Threading::{
        ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetProcessHandleCount, GetProcessTimes, GetSystemTimes, OpenEventW,
        SetEvent, EVENT_MODIFY_STATE,
    };
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::core::{BOOL, HSTRING, Interface, PCWSTR, w};

    /// Must match `src/bench.rs`.
    const EVENTS: [&str; 3] = ["Local\\OmniawareBench.Capture", "Local\\OmniawareBench.ShotFull", "Local\\OmniawareBench.Quit"];
    const CAPTURE: usize = 0;
    const SHOT_FULL: usize = 1;
    const QUIT: usize = 2;
    const RENDERERS: [&str; 2] = ["wgpu", "glow"];
    const VERSION: &str = env!("CARGO_PKG_VERSION");

    static STOP: AtomicBool = AtomicBool::new(false);

    struct Opts {
        hours: f64,
        interval: u64,
        switch_h: f64,
        idle_min: u64,
        scenarios: bool,
        hidden: bool,
        /// Internal: the detached copy `--hidden` starts.
        detached: bool,
    }

    fn opts() -> Result<Opts, String> {
        let mut o = Opts { hours: 72.0, interval: 10, switch_h: 2.0, idle_min: 10, scenarios: true, hidden: false, detached: false };
        let mut a = std::env::args().skip(1);
        while let Some(k) = a.next() {
            let mut num = || a.next().and_then(|v| v.parse::<f64>().ok()).ok_or(format!("{k} needs a number"));
            match k.as_str() {
                "--hours" => o.hours = num()?,
                "--interval" => o.interval = num()?.max(1.0) as u64,
                "--switch" => o.switch_h = num()?.max(0.1),
                "--idle" => o.idle_min = num()? as u64,
                "--no-scenarios" => o.scenarios = false,
                "--hidden" => o.hidden = true,
                "--detached" => o.detached = true,
                "-h" | "--help" => return Err(String::new()),
                _ => return Err(format!("unknown option {k}")),
            }
        }
        Ok(o)
    }

    pub fn run() -> i32 {
        let o = match opts() {
            Ok(o) => o,
            Err(e) => {
                if !e.is_empty() {
                    eprintln!("{e}");
                }
                eprintln!("oa-probe {VERSION}: --hours N (72) --interval S (10) --switch H (2) --idle M (10) --no-scenarios --hidden");
                return 2;
            }
        };
        // --hidden: start a copy with no console at all (nothing for anyone to close by
        // accident) and leave. The copy shows a tray icon that stops it.
        if o.hidden && !o.detached {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--hidden").collect();
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("oa-probe.exe"));
            return match Command::new(exe).args(args).arg("--detached").creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW).spawn() {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("cannot start the hidden probe: {e}");
                    pause();
                    1
                }
            };
        }
        let tray = o.detached.then(tray::start);
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = SetConsoleCtrlHandler(Some(on_ctrl), true);
        }
        let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        let oa = dir.join("omniaware.exe");
        if !oa.exists() {
            eprintln!("omniaware.exe must be in the same folder as oa-probe.exe ({}).", dir.display());
            pause();
            return 1;
        }
        let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into());
        let out = dir.join("bench").join(&host);
        let data = out.join("oa-data");
        if let Err(e) = std::fs::create_dir_all(&data) {
            eprintln!("cannot create {}: {e}", data.display());
            pause();
            return 1;
        }
        let machine = machine_json(&host);
        let _ = std::fs::write(out.join("machine.json"), &machine);
        println!("oa-probe {VERSION} · {host}");
        println!("{}", machine_line());
        println!("Results: {}", out.display());
        println!("Running for {} h. Ctrl+C or closing this window stops it (and its Omniaware).", o.hours);
        println!("The probe starts its own Omniaware, hidden in the tray (\"Omniaware (benchmark)\").");
        println!("Do not start omniaware.exe yourself; the benchmark copy's shortcuts are off on purpose.\n");

        let started = chrono::Local::now();
        let csv_path = out.join(format!("samples-{}.csv", started.format("%Y%m%d-%H%M%S")));
        let mut csv = match std::fs::File::create(&csv_path) {
            Ok(f) => std::io::BufWriter::new(f),
            Err(e) => {
                eprintln!("cannot create {}: {e}", csv_path.display());
                return 1;
            }
        };
        let _ = writeln!(
            csv,
            "time,elapsed_s,renderer,phase,user_idle_s,cpu_ms,cpu_pct_core,sys_cpu_pct,working_set_kb,private_kb,peak_working_set_kb,handles,threads,gpu_dedicated_kb,gpu_shared_kb,note"
        );

        let job = match job() {
            Some(j) => j,
            None => {
                eprintln!("cannot create a job object");
                return 1;
            }
        };
        let gpu = Gpu::new();
        let mut stats = Stats::default();
        let t0 = Instant::now();
        let end = t0 + Duration::from_secs_f64(o.hours * 3600.0);
        let mut period = 0usize;
        let mut last_summary = Instant::now();

        while !STOP.load(SeqCst) && Instant::now() < end {
            let renderer = RENDERERS[period % RENDERERS.len()];
            period += 1;
            let period_end = (Instant::now() + Duration::from_secs_f64(o.switch_h * 3600.0)).min(end);
            write_config(&data, renderer);
            let mut child = match launch(&oa, &data, job) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("cannot start omniaware.exe: {e}");
                    return 1;
                }
            };
            let mut s = Sampler::new(&child);
            let events = open_events();
            let mut note = format!("launch pid {}", child.id());
            if events.is_none() {
                note.push_str(" (no bench events: is this an older omniaware.exe?)");
            }
            let mut next_scenario = Instant::now() + Duration::from_secs(300);
            // Startup is sampled every second for half a minute.
            let mut fast_until = Instant::now() + Duration::from_secs(30);
            loop {
                if STOP.load(SeqCst) || Instant::now() >= period_end {
                    break;
                }
                if let Ok(Some(st)) = child.try_wait() {
                    let r = s.sample(&gpu);
                    emit(&mut csv, &mut stats, t0, renderer, "exited", &r, &format!("exit {st}"));
                    std::thread::sleep(Duration::from_secs(10));
                    break; // relaunch with the same renderer below (period continues)
                }
                let phase = if Instant::now() < fast_until { "startup" } else { "idle" };
                let r = s.sample(&gpu);
                emit(&mut csv, &mut stats, t0, renderer, phase, &r, &std::mem::take(&mut note));
                status(&host, renderer, phase, &r);
                if o.scenarios
                    && let Some(ev) = &events
                    && Instant::now() >= next_scenario
                {
                    if user_idle().as_secs() >= o.idle_min * 60 {
                        scenario(ev, &mut s, &gpu, &mut csv, &mut stats, t0, renderer, &host);
                        next_scenario = Instant::now() + Duration::from_secs(3600);
                        fast_until = Instant::now() + Duration::from_secs(20);
                    } else {
                        next_scenario = Instant::now() + Duration::from_secs(60);
                    }
                }
                let _ = csv.flush();
                if last_summary.elapsed() >= Duration::from_secs(3600) {
                    let _ = std::fs::write(out.join("summary.txt"), stats.summary(&machine_line()));
                    last_summary = Instant::now();
                }
                let wait = if Instant::now() < fast_until { 1 } else { o.interval };
                sleep_unless_stopped(Duration::from_secs(wait));
            }
            // Ask politely, then insist.
            if let Some(ev) = &events {
                signal(ev, QUIT);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(200));
            }
            let _ = child.kill();
            let _ = child.wait();
            if let Some(ev) = events {
                for h in ev {
                    unsafe {
                        let _ = CloseHandle(h);
                    }
                }
            }
            if Instant::now() < period_end && !STOP.load(SeqCst) {
                period -= 1; // crashed early: same renderer again
            }
        }
        let _ = csv.flush();
        let _ = std::fs::write(out.join("summary.txt"), stats.summary(&machine_line()));
        if let Some(t) = tray {
            t.stop();
        }
        println!("\n\nDone. Results in {}", out.display());
        0
    }

    /// The hidden probe's only face: a tray icon whose menu stops the measurement.
    mod tray {
        use std::sync::atomic::Ordering::SeqCst;
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        use tray_icon::{Icon, TrayIconBuilder};
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT};

        pub struct Tray {
            thread: u32,
            join: std::thread::JoinHandle<()>,
        }

        pub fn start() -> Tray {
            let (tx, rx) = std::sync::mpsc::channel();
            let join = std::thread::spawn(move || {
                let _ = tx.send(unsafe { GetCurrentThreadId() });
                let menu = Menu::new();
                let stop = MenuItem::new("Stop benchmark", true, None);
                if menu.append(&stop).is_err() {
                    return;
                }
                // Amber square: easy to tell from Omniaware's own icon.
                let px: Vec<u8> = (0..16 * 16).flat_map(|_| [230u8, 160, 40, 255]).collect();
                let Ok(icon) = Icon::from_rgba(px, 16, 16) else { return };
                let Ok(_tray) = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip("oa-probe: measuring Omniaware (right-click to stop)")
                    .with_icon(icon)
                    .build()
                else {
                    return;
                };
                let id = stop.id().clone();
                MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                    if e.id == id {
                        super::STOP.store(true, SeqCst);
                    }
                }));
                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            });
            Tray { thread: rx.recv().unwrap_or(0), join }
        }

        impl Tray {
            /// Ends the message loop so the icon is removed rather than left behind.
            pub fn stop(self) {
                unsafe {
                    let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
                }
                let _ = self.join.join();
            }
        }
    }

    unsafe extern "system" fn on_ctrl(_t: u32) -> BOOL {
        STOP.store(true, SeqCst);
        BOOL(1)
    }

    fn pause() {
        println!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }

    fn sleep_unless_stopped(d: Duration) {
        let until = Instant::now() + d;
        while Instant::now() < until && !STOP.load(SeqCst) {
            std::thread::sleep(Duration::from_millis(200).min(until.saturating_duration_since(Instant::now())));
        }
    }

    fn write_config(data: &Path, renderer: &str) {
        // Defaults otherwise; version 4 so Omniaware does not rewrite it.
        let _ = std::fs::write(data.join("config.toml"), format!("version = 4\nrenderer = \"{renderer}\"\n"));
    }

    fn launch(oa: &Path, data: &Path, job: HANDLE) -> std::io::Result<Child> {
        let child = Command::new(oa).env("OMNIAWARE_BENCH", "1").env("OMNIAWARE_DATA", data).spawn()?;
        unsafe {
            let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()));
        }
        Ok(child)
    }

    /// Everything started in it dies with the probe, however the probe ends.
    fn job() -> Option<HANDLE> {
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(job)
        }
    }

    /// Omniaware creates the events a moment after it starts.
    fn open_events() -> Option<Vec<HANDLE>> {
        for _ in 0..75 {
            let hs: Vec<HANDLE> =
                EVENTS.iter().filter_map(|n| unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(*n)).ok() }).collect();
            if hs.len() == EVENTS.len() {
                return Some(hs);
            }
            for h in hs {
                unsafe {
                    let _ = CloseHandle(h);
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        None
    }

    fn signal(ev: &[HANDLE], i: usize) {
        unsafe {
            let _ = SetEvent(ev[i]);
        }
    }

    fn user_idle() -> Duration {
        let mut li = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        unsafe {
            if !GetLastInputInfo(&mut li).as_bool() {
                return Duration::ZERO;
            }
            let now = GetTickCount64() as u32;
            Duration::from_millis(now.wrapping_sub(li.dwTime) as u64)
        }
    }

    /// Quick note → main window → hidden → full-screen shot, sampled every second.
    #[allow(clippy::too_many_arguments)]
    fn scenario(
        ev: &[HANDLE],
        s: &mut Sampler,
        gpu: &Gpu,
        csv: &mut impl std::io::Write,
        stats: &mut Stats,
        t0: Instant,
        renderer: &str,
        host: &str,
    ) {
        let steps: [(Option<usize>, &str, u64); 4] = [
            (Some(CAPTURE), "quick_note", 6),
            (Some(CAPTURE), "main_window", 6),
            (Some(CAPTURE), "after_close", 15),
            (Some(SHOT_FULL), "screenshot", 10),
        ];
        for (cmd, phase, secs) in steps {
            if STOP.load(SeqCst) {
                return;
            }
            if let Some(c) = cmd {
                signal(ev, c);
            }
            for i in 0..secs {
                std::thread::sleep(Duration::from_secs(1));
                let r = s.sample(gpu);
                emit(csv, stats, t0, renderer, phase, &r, if i == 0 { "scenario" } else { "" });
                status(host, renderer, phase, &r);
            }
        }
    }

    // ---------- sampling ----------

    #[derive(Default, Clone)]
    struct Reading {
        user_idle_s: u64,
        cpu_ms: f64,
        cpu_pct_core: f64,
        sys_cpu_pct: f64,
        ws_kb: u64,
        private_kb: u64,
        peak_ws_kb: u64,
        handles: u32,
        threads: u32,
        gpu_ded_kb: Option<u64>,
        gpu_shr_kb: Option<u64>,
    }

    struct Sampler {
        h: HANDLE,
        pid: u32,
        last: Option<(Instant, f64)>,
        sys_last: Option<(u64, u64)>,
    }

    fn ft(f: FILETIME) -> u64 {
        (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
    }

    impl Sampler {
        fn new(c: &Child) -> Self {
            Self { h: HANDLE(c.as_raw_handle()), pid: c.id(), last: None, sys_last: None }
        }

        fn sample(&mut self, gpu: &Gpu) -> Reading {
            let mut r = Reading { user_idle_s: user_idle().as_secs(), ..Default::default() };
            unsafe {
                let (mut c, mut e, mut k, mut u) = Default::default();
                if GetProcessTimes(self.h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                    r.cpu_ms = (ft(k) + ft(u)) as f64 / 10_000.0;
                    let now = Instant::now();
                    if let Some((t, ms)) = self.last {
                        let wall = now.duration_since(t).as_secs_f64() * 1000.0;
                        if wall > 0.0 {
                            r.cpu_pct_core = (r.cpu_ms - ms) / wall * 100.0;
                        }
                    }
                    self.last = Some((now, r.cpu_ms));
                }
                let mut m = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
                if GetProcessMemoryInfo(self.h, (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(), m.cb).is_ok() {
                    r.ws_kb = m.WorkingSetSize as u64 / 1024;
                    r.private_kb = m.PrivateUsage as u64 / 1024;
                    r.peak_ws_kb = m.PeakWorkingSetSize as u64 / 1024;
                }
                let mut n = 0u32;
                if GetProcessHandleCount(self.h, &mut n).is_ok() {
                    r.handles = n;
                }
                r.threads = threads_of(self.pid);
                let (mut i, mut k2, mut u2) = Default::default();
                if GetSystemTimes(Some(&mut i), Some(&mut k2), Some(&mut u2)).is_ok() {
                    // Kernel time includes idle time.
                    let (idle, total) = (ft(i), ft(k2) + ft(u2));
                    if let Some((li, lt)) = self.sys_last
                        && total > lt
                    {
                        r.sys_cpu_pct = 100.0 * (1.0 - (idle - li) as f64 / (total - lt) as f64);
                    }
                    self.sys_last = Some((idle, total));
                }
            }
            if let Some((d, s)) = gpu.usage(self.pid) {
                r.gpu_ded_kb = Some(d / 1024);
                r.gpu_shr_kb = Some(s / 1024);
            }
            r
        }
    }

    fn threads_of(pid: u32) -> u32 {
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else { return 0 };
            let mut te = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
            let mut n = 0;
            if Thread32First(snap, &mut te).is_ok() {
                loop {
                    if te.th32OwnerProcessID == pid {
                        n += 1;
                    }
                    if Thread32Next(snap, &mut te).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            n
        }
    }

    /// Per-process GPU memory from the "GPU Process Memory" counters (WDDM 2.x, Windows 10 1709+).
    struct Gpu {
        query: Option<PDH_HQUERY>,
        dedicated: PDH_HCOUNTER,
        shared: PDH_HCOUNTER,
    }

    impl Gpu {
        fn new() -> Self {
            unsafe {
                let mut q = PDH_HQUERY::default();
                let (mut d, mut s) = (PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
                let ok = PdhOpenQueryW(PCWSTR::null(), 0, &mut q) == 0
                    && PdhAddEnglishCounterW(q, w!("\\GPU Process Memory(*)\\Dedicated Usage"), 0, &mut d) == 0
                    && PdhAddEnglishCounterW(q, w!("\\GPU Process Memory(*)\\Shared Usage"), 0, &mut s) == 0;
                Self { query: ok.then_some(q), dedicated: d, shared: s }
            }
        }

        fn usage(&self, pid: u32) -> Option<(u64, u64)> {
            let q = self.query?;
            unsafe {
                if PdhCollectQueryData(q) != 0 {
                    return None;
                }
            }
            let prefix = format!("pid_{pid}_");
            Some((sum_for(self.dedicated, &prefix)?, sum_for(self.shared, &prefix)?))
        }
    }

    fn sum_for(c: PDH_HCOUNTER, prefix: &str) -> Option<u64> {
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, None);
            if r != PDH_MORE_DATA || size == 0 {
                return if r == 0 { Some(0) } else { None };
            }
            // u64-aligned buffer for the item array and the names after it.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, Some(items)) != 0 {
                return None;
            }
            let mut sum = 0u64;
            for i in 0..count as usize {
                let it = &*items.add(i);
                let name = it.szName.to_string().unwrap_or_default();
                if name.starts_with(prefix) && it.FmtValue.CStatus == 0 {
                    sum += it.FmtValue.Anonymous.largeValue.max(0) as u64;
                }
            }
            Some(sum)
        }
    }

    fn emit(csv: &mut impl std::io::Write, stats: &mut Stats, t0: Instant, renderer: &str, phase: &str, r: &Reading, note: &str) {
        let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_default();
        let _ = writeln!(
            csv,
            "{},{},{renderer},{phase},{},{:.1},{:.3},{:.1},{},{},{},{},{},{},{},{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            t0.elapsed().as_secs(),
            r.user_idle_s,
            r.cpu_ms,
            r.cpu_pct_core,
            r.sys_cpu_pct,
            r.ws_kb,
            r.private_kb,
            r.peak_ws_kb,
            r.handles,
            r.threads,
            opt(r.gpu_ded_kb),
            opt(r.gpu_shr_kb),
            note.replace(',', ";"),
        );
        stats.add(renderer, phase, r);
    }

    fn status(host: &str, renderer: &str, phase: &str, r: &Reading) {
        let gpu = r.gpu_ded_kb.map(|d| format!("  gpu {:.1} MB", d as f64 / 1024.0)).unwrap_or_default();
        print!(
            "\r[{host}] {}  {renderer:<4} {phase:<11}  ws {:>6.1} MB  private {:>6.1} MB  cpu {:>6.3} %{gpu}      ",
            chrono::Local::now().format("%H:%M:%S"),
            r.ws_kb as f64 / 1024.0,
            r.private_kb as f64 / 1024.0,
            r.cpu_pct_core
        );
        let _ = std::io::stdout().flush();
    }

    // ---------- summary ----------

    #[derive(Default)]
    struct Agg {
        n: u64,
        cpu: f64,
        cpu_max: f64,
        ws: u64,
        ws_max: u64,
        private: u64,
        private_max: u64,
        gpu: u64,
        gpu_max: u64,
        gpu_n: u64,
    }

    #[derive(Default)]
    struct Stats {
        by: BTreeMap<(String, String), Agg>,
    }

    impl Stats {
        fn add(&mut self, renderer: &str, phase: &str, r: &Reading) {
            let a = self.by.entry((renderer.to_string(), phase.to_string())).or_default();
            a.n += 1;
            a.cpu += r.cpu_pct_core;
            a.cpu_max = a.cpu_max.max(r.cpu_pct_core);
            a.ws += r.ws_kb;
            a.ws_max = a.ws_max.max(r.ws_kb);
            a.private += r.private_kb;
            a.private_max = a.private_max.max(r.private_kb);
            if let Some(g) = r.gpu_ded_kb {
                a.gpu += g;
                a.gpu_max = a.gpu_max.max(g);
                a.gpu_n += 1;
            }
        }

        fn summary(&self, machine: &str) -> String {
            let mb = |kb: u64| kb as f64 / 1024.0;
            let mut s = format!("oa-probe {VERSION}\n{machine}\n\n");
            let _ = writeln!(
                s,
                "{:<5} {:<12} {:>7} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>9} {:>9}",
                "rend", "phase", "samples", "cpu% avg", "cpu% max", "ws MB avg", "ws MB max", "priv avg", "priv max", "gpu avg", "gpu max"
            );
            for ((rend, phase), a) in &self.by {
                let n = a.n.max(1);
                let g = a.gpu_n.max(1);
                let _ = writeln!(
                    s,
                    "{:<5} {:<12} {:>7} {:>10.3} {:>10.3} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>9.1} {:>9.1}",
                    rend,
                    phase,
                    a.n,
                    a.cpu / n as f64,
                    a.cpu_max,
                    mb(a.ws / n),
                    mb(a.ws_max),
                    mb(a.private / n),
                    mb(a.private_max),
                    mb(a.gpu / g),
                    mb(a.gpu_max),
                );
            }
            s.push_str("\ncpu% is of one core. ws = working set, priv = private bytes, gpu = dedicated GPU memory.\n");
            s
        }
    }

    // ---------- the machine ----------

    fn reg_sz(key: PCWSTR, name: PCWSTR) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)).ok().ok()?;
        }
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]).trim().to_string())
    }

    fn reg_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
        let mut v = 0u32;
        let mut len = 4u32;
        unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key, name, RRF_RT_REG_DWORD, None, Some((&mut v as *mut u32).cast()), Some(&mut len))
                .ok()
                .ok()?;
        }
        Some(v)
    }

    /// PROCESSOR_POWER_INFORMATION (not in the windows crate).
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct PowerInfo {
        number: u32,
        max_mhz: u32,
        current_mhz: u32,
        mhz_limit: u32,
        max_idle_state: u32,
        current_idle_state: u32,
    }

    struct Cpu {
        name: String,
        base_mhz: Option<u32>,
        max_mhz: Option<u32>,
        packages: u32,
        cores: u32,
        threads: u32,
    }

    fn cpu() -> Cpu {
        let key = w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0");
        let threads = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
        let mut pi = vec![PowerInfo::default(); threads.max(1) as usize];
        let max_mhz = unsafe {
            CallNtPowerInformation(
                ProcessorInformation,
                None,
                0,
                Some(pi.as_mut_ptr().cast()),
                (pi.len() * size_of::<PowerInfo>()) as u32,
            )
        }
        .is_ok()
        .then(|| pi.iter().map(|p| p.max_mhz).max().unwrap_or(0))
        .filter(|&m| m > 0);
        Cpu {
            name: reg_sz(key, w!("ProcessorNameString")).unwrap_or_default(),
            base_mhz: reg_dword(key, w!("~MHz")),
            max_mhz,
            packages: count_relation(RelationProcessorPackage),
            cores: count_relation(RelationProcessorCore),
            threads,
        }
    }

    fn count_relation(rel: windows::Win32::System::SystemInformation::LOGICAL_PROCESSOR_RELATIONSHIP) -> u32 {
        unsafe {
            let mut len = 0u32;
            let _ = GetLogicalProcessorInformationEx(rel, None, &mut len);
            if len == 0 {
                return 0;
            }
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let p = buf.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
            if GetLogicalProcessorInformationEx(rel, Some(p), &mut len).is_err() {
                return 0;
            }
            let (mut off, mut n) = (0usize, 0u32);
            let base = buf.as_ptr() as *const u8;
            while off < len as usize {
                let item = &*(base.add(off) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX);
                n += 1;
                if item.Size == 0 {
                    break;
                }
                off += item.Size as usize;
            }
            n
        }
    }

    fn memory_mb() -> u64 {
        let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe {
            if GlobalMemoryStatusEx(&mut m).is_ok() { m.ullTotalPhys / (1024 * 1024) } else { 0 }
        }
    }

    struct Win {
        name: String,
        display_version: String,
        build: String,
        ubr: Option<u32>,
        edition: String,
    }

    fn windows_version() -> Win {
        let key = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let build = reg_sz(key, w!("CurrentBuild")).unwrap_or_default();
        let mut name = reg_sz(key, w!("ProductName")).unwrap_or_default();
        // Windows 11 still reports "Windows 10" here.
        if build.parse::<u32>().is_ok_and(|b| b >= 22000) {
            name = name.replace("Windows 10", "Windows 11");
        }
        Win {
            name,
            display_version: reg_sz(key, w!("DisplayVersion")).unwrap_or_default(),
            build,
            ubr: reg_dword(key, w!("UBR")),
            edition: reg_sz(key, w!("EditionID")).unwrap_or_default(),
        }
    }

    struct Adapter {
        name: String,
        vram_mb: u64,
        shared_mb: u64,
        driver: String,
    }

    fn adapters() -> Vec<Adapter> {
        let mut out = Vec::new();
        unsafe {
            let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
            let mut i = 0;
            while let Ok(a) = f.EnumAdapters1(i) {
                i += 1;
                let Ok(d) = a.GetDesc1() else { continue };
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
                let driver = a
                    .CheckInterfaceSupport(&IDXGIDevice::IID)
                    .map(|v| {
                        let v = v as u64;
                        format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
                    })
                    .unwrap_or_default();
                out.push(Adapter {
                    name: String::from_utf16_lossy(&d.Description[..n]),
                    vram_mb: d.DedicatedVideoMemory as u64 / (1024 * 1024),
                    shared_mb: d.SharedSystemMemory as u64 / (1024 * 1024),
                    driver,
                });
            }
        }
        out
    }

    struct Display {
        w: i32,
        h: i32,
        scale_pct: u32,
        hz: u32,
        primary: bool,
    }

    fn displays() -> Vec<Display> {
        unsafe extern "system" fn each(m: HMONITOR, _dc: HDC, _r: *mut RECT, l: LPARAM) -> BOOL {
            unsafe {
                let list = &mut *(l.0 as *mut Vec<Display>);
                let mut mi = MONITORINFOEXW::default();
                mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
                if !GetMonitorInfoW(m, (&mut mi as *mut MONITORINFOEXW).cast::<MONITORINFO>()).as_bool() {
                    return BOOL(1);
                }
                let r = mi.monitorInfo.rcMonitor;
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
                let hz = if EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                    dm.dmDisplayFrequency
                } else {
                    0
                };
                list.push(Display {
                    w: r.right - r.left,
                    h: r.bottom - r.top,
                    scale_pct: dx * 100 / 96,
                    hz,
                    primary: mi.monitorInfo.dwFlags & 1 != 0,
                });
                BOOL(1)
            }
        }
        let mut list: Vec<Display> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut Vec<Display> as isize));
        }
        list
    }

    fn power_plan() -> String {
        unsafe {
            let mut guid = std::ptr::null_mut();
            if PowerGetActiveScheme(None, &mut guid).is_err() || guid.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 128];
            let mut size = (buf.len() * 2) as u32;
            let name = if PowerReadFriendlyName(None, Some(guid), None, None, Some(buf.as_mut_ptr().cast()), &mut size).is_ok() {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf16_lossy(&buf[..n])
            } else {
                String::new()
            };
            let _ = LocalFree(Some(HLOCAL(guid.cast())));
            name
        }
    }

    fn js(s: &str) -> String {
        let mut o = String::with_capacity(s.len() + 2);
        o.push('"');
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(o, "\\u{:04x}", c as u32);
                }
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }

    fn opt_num(v: Option<u32>) -> String {
        v.map(|v| v.to_string()).unwrap_or_else(|| "null".into())
    }

    fn machine_json(host: &str) -> String {
        let c = cpu();
        let w = windows_version();
        let gpus: Vec<String> = adapters()
            .iter()
            .map(|a| {
                format!(
                    "{{\"name\": {}, \"vram_mb\": {}, \"shared_mb\": {}, \"driver\": {}}}",
                    js(&a.name),
                    a.vram_mb,
                    a.shared_mb,
                    js(&a.driver)
                )
            })
            .collect();
        let disp: Vec<String> = displays()
            .iter()
            .map(|d| {
                format!(
                    "{{\"width\": {}, \"height\": {}, \"scale_pct\": {}, \"hz\": {}, \"primary\": {}}}",
                    d.w, d.h, d.scale_pct, d.hz, d.primary
                )
            })
            .collect();
        let uptime_h = unsafe { GetTickCount64() } as f64 / 3_600_000.0;
        format!(
            "{{\n  \"probe\": {},\n  \"commit\": {},\n  \"recorded\": {},\n  \"host\": {},\n  \"cpu\": {{\"name\": {}, \"base_mhz\": {}, \"max_mhz\": {}, \"packages\": {}, \"cores\": {}, \"threads\": {}}},\n  \"memory_mb\": {},\n  \"gpus\": [{}],\n  \"displays\": [{}],\n  \"windows\": {{\"name\": {}, \"version\": {}, \"build\": {}, \"ubr\": {}, \"edition\": {}}},\n  \"power_plan\": {},\n  \"uptime_h\": {:.1}\n}}\n",
            js(VERSION),
            js(option_env!("GITHUB_SHA").unwrap_or("local")),
            js(&chrono::Local::now().to_rfc3339()),
            js(host),
            js(&c.name),
            opt_num(c.base_mhz),
            opt_num(c.max_mhz),
            c.packages,
            c.cores,
            c.threads,
            memory_mb(),
            gpus.join(", "),
            disp.join(", "),
            js(&w.name),
            js(&w.display_version),
            js(&w.build),
            opt_num(w.ubr),
            js(&w.edition),
            js(&power_plan()),
            uptime_h,
        )
    }

    /// One line for the console and the summary.
    fn machine_line() -> String {
        let c = cpu();
        let w = windows_version();
        let gpu = adapters().first().map(|a| a.name.clone()).unwrap_or_default();
        format!(
            "{} ({}C/{}T, {}–{} MHz) · {} GB · {} · {} {} ({}.{})",
            c.name,
            c.cores,
            c.threads,
            c.base_mhz.unwrap_or(0),
            c.max_mhz.unwrap_or(0),
            memory_mb().div_ceil(1024),
            gpu,
            w.name,
            w.display_version,
            w.build,
            w.ubr.unwrap_or(0)
        )
    }
}
