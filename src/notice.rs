//! "Saved" notice above the system tray: a small native popup, independent of the egui
//! window (which is hidden by the time it shows). It never takes focus; hovering keeps it,
//! clicking opens the entry. Errors stay until clicked.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Ok,
    Error,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Notice {
    pub kind: Kind,
    /// "Saved", "Captured silently", "Couldn't save"
    pub title: String,
    /// "Journal · Mon 5 Oct"
    pub context: String,
    /// "42 words · 1 image"
    pub detail: String,
    /// Entry the click opens.
    pub entry: Option<i64>,
}

impl Notice {
    pub fn ok(title: &str, context: String, detail: String, entry: Option<i64>) -> Self {
        Self { kind: Kind::Ok, title: title.into(), context, detail, entry }
    }

    pub fn error(detail: String) -> Self {
        Self {
            kind: Kind::Error,
            title: crate::text::NOTICE_FAILED.into(),
            context: crate::text::NOTICE_SEE_LOG.into(),
            detail,
            entry: None,
        }
    }
}

/// "42 words · 1 image · 270.7 kB"
pub fn detail(words: usize, images: usize, bytes: u64) -> String {
    use crate::text::plural;
    let mut parts = Vec::new();
    if words > 0 {
        parts.push(format!("{words} {}", plural(words, "word", "words")));
    }
    if images > 0 {
        let (v, u) = crate::doc::fmt_size(bytes);
        parts.push(format!("{images} {} · {v} {u}", plural(images, "image", "images")));
    }
    parts.join(" · ")
}

#[cfg(windows)]
mod imp {
    use super::{Kind, Notice};
    use std::cell::RefCell;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
    use windows::Win32::Graphics::Dwm::{DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    use windows::Win32::Graphics::Gdi::{
        AddFontMemResourceEx, BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
        CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_NOPREFIX, DT_RIGHT,
        DT_SINGLELINE, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetTextExtentPoint32W, HDC,
        HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor,
        TRANSPARENT, TextOutW,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, HWND_TOPMOST, IDC_HAND,
        KillTimer, LWA_ALPHA, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, RegisterClassW, SPI_GETWORKAREA,
        SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetLayeredWindowAttributes,
        SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage, WM_APP, WM_ERASEBKGND,
        WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };
    use windows::core::w;

    const WM_SHOW: u32 = WM_APP + 1;
    /// From UI/Controls, which this crate does not otherwise need.
    const WM_MOUSELEAVE: u32 = 0x02A3;
    const TICK: usize = 1;

    const FADE_IN: f32 = 0.15;
    const HOLD: f32 = 3.2;
    const FADE_OUT: f32 = 0.6;
    const FULL: f32 = 245.0;
    /// Logical size of the popup.
    const W: f32 = 340.0;
    const H: f32 = 64.0;

    static HWND_: OnceLock<isize> = OnceLock::new();
    static PENDING: Mutex<Option<Notice>> = Mutex::new(None);
    static ON_CLICK: OnceLock<Box<dyn Fn(i64) + Send + Sync>> = OnceLock::new();

    #[derive(PartialEq)]
    enum Phase {
        Hidden,
        In,
        Hold,
        Out,
    }

    struct State {
        cur: Option<Notice>,
        phase: Phase,
        /// Start of the current phase.
        t0: Instant,
        alpha: f32,
        hover: bool,
        scale: f32,
        fonts: Option<[HFONT; 3]>,
    }

    thread_local! {
        static ST: RefCell<State> = RefCell::new(State {
            cur: None,
            phase: Phase::Hidden,
            t0: Instant::now(),
            alpha: 0.0,
            hover: false,
            scale: 0.0,
            fonts: None,
        });
    }

    fn rgb(c: [u8; 3]) -> COLORREF {
        COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    pub fn init(on_click: impl Fn(i64) + Send + Sync + 'static) {
        if ON_CLICK.set(Box::new(on_click)).is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            // Private copies of the app fonts, so the notice matches the UI.
            for data in [
                &include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")[..],
                &include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")[..],
            ] {
                let n = 0u32;
                let _ = AddFontMemResourceEx(data.as_ptr().cast(), data.len() as u32, None, &n);
            }
            let Ok(module) = GetModuleHandleW(None) else { return };
            let inst: HINSTANCE = module.into();
            let class = w!("OmniawareNotice");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc_),
                hInstance: inst,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = match CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                class,
                w!("Omniaware"),
                WS_POPUP,
                0,
                0,
                10,
                10,
                None,
                None,
                Some(inst),
                None,
            ) {
                Ok(h) => h,
                Err(e) => return crate::log::error(format!("notice window: {e}")),
            };
            let pref = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&pref as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            let _ = HWND_.set(hwnd.0 as isize);
            // A notice may have been queued before the window existed.
            if PENDING.lock().map(|p| p.is_some()).unwrap_or(false) {
                let _ = PostMessageW(Some(hwnd), WM_SHOW, WPARAM(0), LPARAM(0));
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    pub fn show(n: Notice) {
        if let Ok(mut p) = PENDING.lock() {
            *p = Some(n);
        }
        if let Some(&h) = HWND_.get() {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_SHOW, WPARAM(0), LPARAM(0));
            }
        }
    }

    unsafe extern "system" fn proc_(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_SHOW => {
                    let n = PENDING.lock().ok().and_then(|mut p| p.take());
                    if let Some(n) = n {
                        ST.with_borrow_mut(|s| {
                            s.cur = Some(n);
                            // Already up: refresh in place, no new fade-in.
                            s.phase = if s.phase == Phase::Hidden { Phase::In } else { Phase::Hold };
                            s.t0 = Instant::now();
                        });
                        place(h);
                        let _ = InvalidateRect(Some(h), None, false);
                        SetTimer(Some(h), TICK, 16, None);
                    }
                    LRESULT(0)
                }
                WM_TIMER => {
                    tick(h);
                    LRESULT(0)
                }
                WM_MOUSEMOVE => {
                    let first = ST.with_borrow_mut(|s| {
                        let first = !s.hover;
                        s.hover = true;
                        if s.phase == Phase::Out {
                            s.phase = Phase::Hold;
                        }
                        first
                    });
                    if first {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: h,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    LRESULT(0)
                }
                WM_MOUSELEAVE => {
                    ST.with_borrow_mut(|s| {
                        s.hover = false;
                        if s.phase == Phase::Hold {
                            s.t0 = Instant::now(); // full hold time again after the pointer leaves
                        }
                    });
                    LRESULT(0)
                }
                WM_LBUTTONUP => {
                    let entry = ST.with_borrow(|s| s.cur.as_ref().and_then(|n| n.entry));
                    hide(h);
                    if let (Some(id), Some(f)) = (entry, ON_CLICK.get()) {
                        f(id);
                    }
                    LRESULT(0)
                }
                WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
                WM_ERASEBKGND => LRESULT(1),
                WM_PAINT => {
                    paint(h);
                    LRESULT(0)
                }
                _ => DefWindowProcW(h, msg, w, l),
            }
        }
    }

    /// Bottom-right corner of the primary work area (above the tray), DPI-scaled.
    unsafe fn place(h: HWND) {
        unsafe {
            let scale = GetDpiForWindow(h).max(96) as f32 / 96.0;
            ST.with_borrow_mut(|s| {
                if s.scale != scale || s.fonts.is_none() {
                    if let Some(f) = s.fonts.take() {
                        for x in f {
                            let _ = DeleteObject(x.into());
                        }
                    }
                    s.fonts = Some([font(13.5 * scale, 700), font(12.5 * scale, 400), font(11.5 * scale, 400)]);
                    s.scale = scale;
                }
            });
            let mut wa = RECT::default();
            let _ = SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some((&mut wa as *mut RECT).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let (w, hh, m) = ((W * scale) as i32, (H * scale) as i32, (14.0 * scale) as i32);
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                wa.right - w - m,
                wa.bottom - hh - m,
                w,
                hh,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            set_alpha(h, ST.with_borrow(|s| s.alpha));
        }
    }

    unsafe fn font(px: f32, weight: i32) -> HFONT {
        unsafe {
            CreateFontW(
                -(px.round() as i32),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("JetBrains Mono"),
            )
        }
    }

    unsafe fn set_alpha(h: HWND, a: f32) {
        unsafe {
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), a.clamp(0.0, 255.0) as u8, LWA_ALPHA);
        }
    }

    unsafe fn hide(h: HWND) {
        unsafe {
            let _ = KillTimer(Some(h), TICK);
            let _ = ShowWindow(h, SW_HIDE);
        }
        ST.with_borrow_mut(|s| {
            s.phase = Phase::Hidden;
            s.alpha = 0.0;
            s.hover = false;
        });
    }

    unsafe fn tick(h: HWND) {
        let (alpha, done) = ST.with_borrow_mut(|s| {
            let el = s.t0.elapsed().as_secs_f32();
            let error = s.cur.as_ref().is_some_and(|n| n.kind == Kind::Error);
            match s.phase {
                Phase::In => {
                    s.alpha = FULL * (el / FADE_IN).min(1.0);
                    if el >= FADE_IN {
                        s.phase = Phase::Hold;
                        s.t0 = Instant::now();
                    }
                }
                Phase::Hold => {
                    s.alpha = FULL;
                    // Errors wait for a click; hovering pauses the countdown.
                    if !error && !s.hover && el >= HOLD {
                        s.phase = Phase::Out;
                        s.t0 = Instant::now();
                    }
                }
                Phase::Out => s.alpha = FULL * (1.0 - el / FADE_OUT).max(0.0),
                Phase::Hidden => {}
            }
            (s.alpha, s.phase == Phase::Out && el >= FADE_OUT)
        });
        unsafe {
            if done {
                hide(h);
            } else {
                set_alpha(h, alpha);
            }
        }
    }

    unsafe fn paint(h: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let (w, hh) = (rc.right - rc.left, rc.bottom - rc.top);
            // Double-buffered to avoid flicker while fading.
            let mem = CreateCompatibleDC(Some(dc));
            let bmp = CreateCompatibleBitmap(dc, w, hh);
            let old_bmp = SelectObject(mem, bmp.into());
            ST.with_borrow(|s| draw(mem, &rc, s));
            let _ = BitBlt(dc, 0, 0, w, hh, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old_bmp);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            let _ = EndPaint(h, &ps);
        }
    }

    unsafe fn draw(dc: HDC, rc: &RECT, s: &State) {
        let Some(n) = &s.cur else { return };
        let Some([bold, regular, small]) = s.fonts else { return };
        let k = s.scale;
        let px = |v: f32| (v * k).round() as i32;
        // Colours come from the active theme at paint time.
        let pal = crate::theme::p();
        let c = crate::theme::rgb3;
        let accent = if n.kind == Kind::Ok { c(pal.ok) } else { c(pal.err) };
        unsafe {
            let fill = |r: RECT, c: [u8; 3]| {
                let b = CreateSolidBrush(rgb(c));
                FillRect(dc, &r, b);
                let _ = DeleteObject(b.into());
            };
            fill(*rc, c(pal.bg));
            let b = CreateSolidBrush(rgb(c(pal.key_stroke)));
            FrameRect(dc, rc, b);
            let _ = DeleteObject(b.into());
            fill(RECT { left: rc.left, top: rc.top, right: rc.left + px(3.0), bottom: rc.bottom }, accent);

            SetBkMode(dc, TRANSPARENT);
            let x0 = px(16.0);
            let y1 = px(11.0);
            // line 1: "✓ Saved" (bold, green/red) + " · Journal · Mon 5 Oct" (dim)
            let mark = if n.kind == Kind::Ok { "✓ " } else { "! " };
            let title = wide(&format!("{mark}{}", n.title));
            SelectObject(dc, bold.into());
            SetTextColor(dc, rgb(accent));
            let _ = TextOutW(dc, x0, y1, &title);
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &title, &mut sz);
            let hint = if n.entry.is_some() { "open ↗" } else if n.kind == Kind::Error { "dismiss" } else { "" };
            SelectObject(dc, small.into());
            SetTextColor(dc, rgb(c(pal.faint)));
            let mut hint_w = wide(hint);
            let mut hr = RECT { left: rc.left, top: y1 + px(1.0), right: rc.right - px(12.0), bottom: y1 + px(20.0) };
            DrawTextW(dc, &mut hint_w, &mut hr, DT_RIGHT | DT_SINGLELINE | DT_NOPREFIX);
            if !n.context.is_empty() {
                SelectObject(dc, regular.into());
                SetTextColor(dc, rgb(c(pal.weak)));
                let mut ctx = wide(&format!(" · {}", n.context));
                let mut cr = RECT { left: x0 + sz.cx, top: y1 + px(1.0), right: rc.right - px(64.0), bottom: y1 + px(20.0) };
                DrawTextW(dc, &mut ctx, &mut cr, DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
            }
            // line 2: details
            SelectObject(dc, small.into());
            SetTextColor(dc, rgb(c(pal.soft)));
            let mut d = wide(&n.detail);
            let mut dr = RECT { left: x0, top: px(36.0), right: rc.right - px(12.0), bottom: rc.bottom - px(6.0) };
            DrawTextW(dc, &mut d, &mut dr, DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Notice;
    pub fn init(_on_click: impl Fn(i64) + Send + Sync + 'static) {}
    pub fn show(n: Notice) {
        eprintln!("[notice] {} · {} · {}", n.title, n.context, n.detail);
    }
}

pub use imp::{init, show};
