//! Screenshots. `Ctrl+Alt+S` freezes every screen under a dimmed overlay: drag an area, click a
//! window (the desktop means its whole screen), `Enter` for the screen under the pointer, `Esc`
//! or a right-click to cancel. `Ctrl+Alt+Shift+S` takes the screen under the pointer at once,
//! with nothing shown. The overlay never takes focus, so a game in borderless full screen keeps it.
//!
//! Each shot is stored as a PNG in the blob store, copied to `Pictures\OA Screenshots` and put on
//! the clipboard; the app then files it in today's journal.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

/// Folder under Pictures.
pub const FOLDER: &str = "OA Screenshots";

pub enum Outcome {
    Saved(Saved),
    Cancelled,
    Failed(String),
}

pub struct Saved {
    /// BLAKE3 prefix from `blob::store_rgba`; the attachment ref is `<hash>.png`.
    pub hash: String,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    /// The copy in Pictures\OA Screenshots, or why it couldn't be written.
    pub file: Result<PathBuf, String>,
    /// Why the clipboard couldn't be set (the shot is saved regardless).
    pub clipboard: Option<String>,
}

/// One shot at a time; a second press while the overlay is up is ignored.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Takes a shot on its own thread and calls `done` there when it is saved, cancelled or failed.
pub fn start(full: bool, data_dir: PathBuf, done: impl FnOnce(Outcome) + Send + 'static) {
    if BUSY.swap(true, SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let when = chrono::Local::now();
        let out = match imp::grab(full) {
            Ok(Some(f)) => save(&data_dir, f, when),
            Ok(None) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(format!("screenshot: {e}")),
        };
        BUSY.store(false, SeqCst);
        done(out);
    });
}

/// Captured pixels, top-down BGRA (as GDI hands them over).
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

fn save(data_dir: &Path, f: Frame, when: chrono::DateTime<chrono::Local>) -> Outcome {
    let Frame { w, h, mut px } = f;
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    let hash = match crate::blob::store_rgba(data_dir, w, h, &px) {
        Ok(h) => h,
        Err(e) => return Outcome::Failed(format!("screenshot: {e}")),
    };
    let src = crate::blob::path_for(data_dir, &hash, "png");
    let bytes = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    let file = imp::pictures().ok_or_else(|| "Pictures folder not found".to_string()).and_then(|pics| {
        let dir = pics.join(FOLDER);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let dst = free_path(&dir, &file_name(when));
        std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        Ok(dst)
    });
    let clipboard = arboard::Clipboard::new()
        .and_then(|mut c| c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: px.into() }))
        .err()
        .map(|e| e.to_string());
    Outcome::Saved(Saved { hash, w, h, bytes, file, clipboard })
}

/// "OA 2026-10-12 14.32.07.png"
pub fn file_name(t: chrono::DateTime<chrono::Local>) -> String {
    format!("OA {}.png", t.format("%Y-%m-%d %H.%M.%S"))
}

/// `name` in `dir`, or "<stem> (2).png", "(3)", … if taken (two shots within one second).
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).png")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Rows `y0..y1`, columns `x0..x1` of a top-down 4-byte-per-pixel image `w` wide.
fn crop(px: &[u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    let (stride, row) = (w as usize * 4, (x1 - x0) as usize * 4);
    let mut out = Vec::with_capacity(row * (y1 - y0) as usize);
    for y in y0..y1 {
        let s = y as usize * stride + x0 as usize * 4;
        out.extend_from_slice(&px[s..s + row]);
    }
    out
}

#[cfg(windows)]
mod imp {
    use super::Frame;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicIsize, Ordering::SeqCst};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_CLOAK, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_TRANSITIONS_FORCEDISABLED, DwmGetWindowAttribute,
        DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CAPTUREBLT, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
        CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, EnumDisplayMonitors, FillRect, FrameRect, GetDC, GetDIBits,
        GetMonitorInfoW, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, HGDIOBJ, HMONITOR, InvalidateRect, MONITOR_DEFAULTTONEAREST,
        MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject, SetBkMode,
        SetTextColor, TRANSPARENT, TextOutW, UpdateWindow,
    };
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN};
    use windows::Win32::UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GWL_EXSTYLE,
        GetClassNameW, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
        HHOOK, HWND_TOPMOST, IDC_CROSS, IsIconic, IsWindowVisible, KBDLLHOOKSTRUCT, LoadCursorW, MA_NOACTIVATE, MSG,
        PostMessageW, PostQuitMessage, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, SetWindowsHookExW, ShowWindow,
        TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
        WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_SYSKEYDOWN,
        WM_SYSKEYUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::{BOOL, w};

    const WM_CANCEL: u32 = WM_APP + 1;
    const WM_SCREEN: u32 = WM_APP + 2;
    /// A press that moves less than this is a click (window / screen), not a drag.
    const DRAG: i32 = 4;

    pub fn pictures() -> Option<PathBuf> {
        unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        }
    }

    pub fn grab(full: bool) -> Result<Option<Frame>, String> {
        // Physical pixels whatever the scaling, on every monitor.
        unsafe {
            let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        if full {
            return grab_rect(monitor_at_cursor()).map(Some);
        }
        let vs = virtual_screen();
        let frozen = grab_rect(vs)?;
        let Some(r) = overlay(vs, &frozen)? else { return Ok(None) };
        let (x0, y0) = (r.left.clamp(0, frozen.w as i32) as u32, r.top.clamp(0, frozen.h as i32) as u32);
        let (x1, y1) = (r.right.clamp(0, frozen.w as i32) as u32, r.bottom.clamp(0, frozen.h as i32) as u32);
        if x1 <= x0 || y1 <= y0 {
            return Ok(None);
        }
        Ok(Some(Frame { w: x1 - x0, h: y1 - y0, px: super::crop(&frozen.px, frozen.w, x0, y0, x1, y1) }))
    }

    fn virtual_screen() -> RECT {
        unsafe {
            let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            RECT { left: x, top: y, right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN), bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN) }
        }
    }

    fn cursor() -> POINT {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        pt
    }

    fn monitor_at_cursor() -> RECT {
        unsafe {
            let mon = MonitorFromPoint(cursor(), MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcMonitor } else { virtual_screen() }
        }
    }

    fn bmi(w: i32, h: i32) -> BITMAPINFO {
        BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// The screen pixels in `r` (screen coordinates).
    fn grab_rect(r: RECT) -> Result<Frame, String> {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            return Err("no screen area".into());
        }
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("GetDC failed".into());
            }
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp.into());
            // CAPTUREBLT: include layered windows (menus, tooltips, notices).
            let blt = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0));
            SelectObject(mem, old);
            let mut px = vec![0u8; w as usize * h as usize * 4];
            let mut bi = bmi(w, h);
            let lines = if blt.is_ok() {
                GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS)
            } else {
                0
            };
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            blt.map_err(|e| format!("BitBlt: {e}"))?;
            if lines == 0 {
                return Err("GetDIBits failed".into());
            }
            Ok(Frame { w: w as u32, h: h as u32, px })
        }
    }

    // ---------- the overlay ----------

    /// Window handle of the open overlay, for the keyboard hook.
    static OVERLAY: AtomicIsize = AtomicIsize::new(0);

    /// A memory DC with its bitmap selected, freed together.
    struct Surface {
        dc: HDC,
        bmp: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }

    /// A DIB-backed surface holding `px` (top-down BGRA), each channel scaled by `k`/256.
    fn surface_from(px: &[u8], w: i32, h: i32, k: u32) -> Option<Surface> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi(w, h), DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, px.len());
            if k >= 256 {
                dst.copy_from_slice(px);
            } else {
                for (d, s) in dst.iter_mut().zip(px) {
                    *d = ((*s as u32 * k) >> 8) as u8;
                }
            }
            let old = SelectObject(dc, bmp.into());
            Some(Surface { dc, bmp, old })
        }
    }

    struct Ov {
        w: i32,
        h: i32,
        bright: Surface,
        dim: Surface,
        back: Surface,
        /// Hit targets in client coordinates: windows front to back.
        windows: Vec<RECT>,
        monitors: Vec<RECT>,
        /// Screen position of the overlay's client origin.
        origin: POINT,
        sel: Option<RECT>,
        down: Option<POINT>,
        dragging: bool,
        /// Some(rect) = take it, None = cancelled. Unset while the overlay is open.
        result: Option<Option<RECT>>,
        char_w: i32,
        line_h: i32,
        accent: COLORREF,
        label_bg: COLORREF,
        border: i32,
        /// Last field: dropped after the DCs it is selected into.
        _font: Font,
    }

    struct Font(HFONT);
    impl Drop for Font {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }

    thread_local! {
        static OV: RefCell<Option<Ov>> = const { RefCell::new(None) };
    }

    fn rgb(c: [u8; 3]) -> COLORREF {
        COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn contains(r: &RECT, p: POINT) -> bool {
        p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
    }

    fn inflate(r: RECT, d: i32) -> RECT {
        RECT { left: r.left - d, top: r.top - d, right: r.right + d, bottom: r.bottom + d }
    }

    fn intersect(a: &RECT, b: &RECT) -> Option<RECT> {
        let r = RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) };
        (r.right > r.left && r.bottom > r.top).then_some(r)
    }

    fn class_is(h: HWND, names: &[&str]) -> bool {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
        let class = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        names.contains(&class.as_str())
    }

    /// Visible top-level windows worth clicking, front to back, in screen coordinates.
    fn windows_on_screen() -> Vec<RECT> {
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
            unsafe {
                let list = &mut *(l.0 as *mut Vec<RECT>);
                if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                    return BOOL(1);
                }
                let mut cloaked = 0u32;
                let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4);
                let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
                let untitled_tool = ex & WS_EX_TOOLWINDOW.0 != 0 && GetWindowTextLengthW(h) == 0;
                // The desktop itself is handled as "its whole screen"; click-through layers
                // (game and GPU overlays) are never what the pointer means.
                if cloaked != 0
                    || ex & WS_EX_TRANSPARENT.0 != 0
                    || untitled_tool
                    || class_is(h, &["Progman", "WorkerW"])
                {
                    return BOOL(1);
                }
                let mut r = RECT::default();
                if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut r as *mut RECT).cast(), size_of::<RECT>() as u32)
                    .is_err()
                {
                    let _ = GetWindowRect(h, &mut r);
                }
                if r.right > r.left && r.bottom > r.top {
                    list.push(r);
                }
                BOOL(1)
            }
        }
        let mut list: Vec<RECT> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut list as *mut Vec<RECT> as isize));
        }
        list
    }

    fn monitors() -> Vec<RECT> {
        unsafe extern "system" fn each(_m: HMONITOR, _dc: HDC, r: *mut RECT, l: LPARAM) -> BOOL {
            unsafe {
                (*(l.0 as *mut Vec<RECT>)).push(*r);
            }
            BOOL(1)
        }
        let mut list: Vec<RECT> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut Vec<RECT> as isize));
        }
        list
    }

    fn to_client(r: RECT, o: POINT) -> RECT {
        RECT { left: r.left - o.x, top: r.top - o.y, right: r.right - o.x, bottom: r.bottom - o.y }
    }

    /// Shows the frozen screen and returns what was chosen (client = virtual-screen coordinates).
    fn overlay(vs: RECT, frozen: &Frame) -> Result<Option<RECT>, String> {
        let (w, h) = (frozen.w as i32, frozen.h as i32);
        let origin = POINT { x: vs.left, y: vs.top };
        let whole = RECT { left: 0, top: 0, right: w, bottom: h };
        let windows: Vec<RECT> =
            windows_on_screen().into_iter().filter_map(|r| intersect(&to_client(r, origin), &whole)).collect();
        let mut mons: Vec<RECT> = monitors().into_iter().filter_map(|r| intersect(&to_client(r, origin), &whole)).collect();
        if mons.is_empty() {
            mons.push(whole);
        }
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = w!("OmniawareScreenshot");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc_),
                hInstance: module.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly from the second shot on
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class,
                w!("Omniaware screenshot"),
                WS_POPUP,
                vs.left,
                vs.top,
                w,
                h,
                None,
                None,
                Some(module.into()),
                None,
            )
            .map_err(|e| format!("overlay window: {e}"))?;

            let pal = crate::theme::p();
            let c3 = crate::theme::rgb3;
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let font = CreateFontW(
                -((12.5 * scale).round() as i32),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("JetBrains Mono"),
            );
            let (Some(bright), Some(dim), Some(back)) = (
                surface_from(&frozen.px, w, h, 256),
                surface_from(&frozen.px, w, h, 130),
                surface_from(&frozen.px, w, h, 256),
            ) else {
                let _ = DeleteObject(font.into());
                let _ = DestroyWindow(hwnd);
                return Err("not enough memory for the overlay".into());
            };
            SelectObject(dim.dc, font.into());
            SelectObject(back.dc, font.into());
            let mut sz = SIZE::default();
            let _ = GetTextExtentPoint32W(back.dc, &wide("0"), &mut sz);
            let mut ov = Ov {
                w,
                h,
                bright,
                dim,
                back,
                windows,
                monitors: mons,
                origin,
                sel: None,
                down: None,
                dragging: false,
                result: None,
                char_w: sz.cx.max(1),
                line_h: sz.cy.max(1),
                accent: rgb(c3(pal.accent)),
                label_bg: rgb(c3(pal.bg)),
                border: (2.0 * scale).round().max(1.0) as i32,
                _font: Font(font),
            };
            hint(&ov, rgb(c3(pal.text)), scale);
            let p = cursor();
            ov.sel = Some(target(&ov, POINT { x: p.x - origin.x, y: p.y - origin.y }));
            OV.with_borrow_mut(|o| *o = Some(ov));

            // Shown cloaked and painted once before DWM draws it: no flash of an empty window.
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, (&on as *const i32).cast(), 4);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), vs.left, vs.top, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(hwnd);
            let off: i32 = 0;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, (&off as *const i32).cast(), 4);

            OVERLAY.store(hwnd.0 as isize, SeqCst);
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keys), Some(module.into()), 0).ok();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Some(hk) = hook {
                let _ = UnhookWindowsHookEx(hk);
            }
            OVERLAY.store(0, SeqCst);
            let ov = OV.with_borrow_mut(|o| o.take());
            Ok(ov.and_then(|o| o.result.flatten()))
        }
    }

    /// What a click at `p` would take: the window in front there, else that whole screen.
    fn target(o: &Ov, p: POINT) -> RECT {
        o.windows
            .iter()
            .chain(o.monitors.iter())
            .find(|r| contains(r, p))
            .copied()
            .unwrap_or(RECT { left: 0, top: 0, right: o.w, bottom: o.h })
    }

    fn screen_at(o: &Ov, p: POINT) -> RECT {
        o.monitors.iter().find(|r| contains(r, p)).copied().unwrap_or(RECT { left: 0, top: 0, right: o.w, bottom: o.h })
    }

    /// One-line help at the top of the screen under the pointer, baked into the dimmed layer.
    fn hint(o: &Ov, text: COLORREF, scale: f32) {
        let p = cursor();
        let mon = screen_at(o, POINT { x: p.x - o.origin.x, y: p.y - o.origin.y });
        let s = wide(crate::text::SHOT_HINT);
        let pad = (10.0 * scale) as i32;
        let tw = o.char_w * s.len() as i32;
        let x = mon.left + ((mon.right - mon.left) - tw) / 2;
        let y = mon.top + (24.0 * scale) as i32;
        let bg = RECT { left: x - pad, top: y - pad / 2, right: x + tw + pad, bottom: y + o.line_h + pad / 2 };
        unsafe {
            let b = CreateSolidBrush(o.label_bg);
            FillRect(o.dim.dc, &bg, b);
            let _ = DeleteObject(b.into());
            SetBkMode(o.dim.dc, TRANSPARENT);
            SetTextColor(o.dim.dc, text);
            let _ = TextOutW(o.dim.dc, x, y, &s);
        }
    }

    /// The "1920 × 1080" tag: above the selection, or inside it at the top edge of the screen.
    fn label(o: &Ov, s: &RECT) -> (RECT, Vec<u16>) {
        let t = wide(&format!("{} × {}", s.right - s.left, s.bottom - s.top));
        let pad = o.char_w / 2 + 2;
        let (lw, lh) = (o.char_w * t.len() as i32 + 2 * pad, o.line_h + pad);
        let gap = o.border + 2;
        let top = if s.top - lh - gap >= 0 { s.top - lh - gap } else { s.top + gap };
        let left = s.left.clamp(0, (o.w - lw).max(0));
        (RECT { left, top, right: left + lw, bottom: top + lh }, t)
    }

    /// Everything a selection paints: the frame around it and its label.
    fn dirty(o: &Ov, s: &Option<RECT>) -> Vec<RECT> {
        match s {
            Some(s) => vec![inflate(*s, o.border + 1), label(o, s).0],
            None => Vec::new(),
        }
    }

    fn set_sel(h: HWND, o: &mut Ov, s: RECT) {
        if o.sel == Some(s) {
            return;
        }
        let mut areas = dirty(o, &o.sel);
        o.sel = Some(s);
        areas.extend(dirty(o, &o.sel));
        unsafe {
            for r in areas {
                let _ = InvalidateRect(Some(h), Some(&r), false);
            }
        }
    }

    fn finish(h: HWND, r: Option<RECT>) {
        OV.with_borrow_mut(|o| {
            if let Some(o) = o.as_mut()
                && o.result.is_none()
            {
                o.result = Some(r);
            }
        });
        unsafe {
            let _ = DestroyWindow(h);
        }
    }

    fn point(l: LPARAM) -> POINT {
        POINT { x: (l.0 & 0xffff) as i16 as i32, y: ((l.0 >> 16) & 0xffff) as i16 as i32 }
    }

    unsafe extern "system" fn proc_(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            // Spanning screens with different scaling must not resize or move the overlay.
            WM_DPICHANGED => LRESULT(0),
            WM_PAINT => {
                paint(h);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let p = point(l);
                OV.with_borrow_mut(|o| {
                    let Some(o) = o.as_mut() else { return };
                    let s = match o.down {
                        Some(d) if o.dragging || (p.x - d.x).abs() > DRAG || (p.y - d.y).abs() > DRAG => {
                            o.dragging = true;
                            RECT { left: d.x.min(p.x), top: d.y.min(p.y), right: d.x.max(p.x) + 1, bottom: d.y.max(p.y) + 1 }
                        }
                        Some(_) => return,
                        None => target(o, p),
                    };
                    set_sel(h, o, s);
                });
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                OV.with_borrow_mut(|o| {
                    if let Some(o) = o.as_mut() {
                        o.down = Some(point(l));
                        o.dragging = false;
                    }
                });
                unsafe {
                    SetCapture(h);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = unsafe { ReleaseCapture() };
                let p = point(l);
                let r = OV.with_borrow(|o| o.as_ref().map(|o| if o.dragging { o.sel } else { Some(target(o, p)) }));
                finish(h, r.flatten());
                LRESULT(0)
            }
            WM_RBUTTONUP | WM_CANCEL => {
                finish(h, None);
                LRESULT(0)
            }
            WM_SCREEN => {
                let p = cursor();
                let r = OV.with_borrow(|o| o.as_ref().map(|o| screen_at(o, POINT { x: p.x - o.origin.x, y: p.y - o.origin.y })));
                finish(h, r);
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(h, msg, w, l) },
        }
    }

    fn paint(h: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            OV.with_borrow(|o| {
                let Some(o) = o.as_ref() else { return };
                let c = ps.rcPaint;
                let blt = |dst: HDC, r: RECT, src: HDC| {
                    let _ = BitBlt(dst, r.left, r.top, r.right - r.left, r.bottom - r.top, Some(src), r.left, r.top, SRCCOPY);
                };
                blt(o.back.dc, c, o.dim.dc);
                if let Some(s) = o.sel {
                    if let Some(i) = intersect(&s, &c) {
                        blt(o.back.dc, i, o.bright.dc);
                    }
                    let b = CreateSolidBrush(o.accent);
                    for k in 1..=o.border {
                        FrameRect(o.back.dc, &inflate(s, k), b);
                    }
                    let _ = DeleteObject(b.into());
                    let (lr, t) = label(o, &s);
                    let bg = CreateSolidBrush(o.label_bg);
                    FillRect(o.back.dc, &lr, bg);
                    let _ = DeleteObject(bg.into());
                    SetBkMode(o.back.dc, TRANSPARENT);
                    SetTextColor(o.back.dc, o.accent);
                    let pad = o.char_w / 2 + 2;
                    let _ = TextOutW(o.back.dc, lr.left + pad, lr.top + pad / 2, &t);
                }
                blt(dc, c, o.back.dc);
            });
            let _ = EndPaint(h, &ps);
        }
    }

    /// Esc cancels, Enter takes the screen under the pointer. The overlay has no focus (the app
    /// in front keeps it), so keys come through a low-level hook; these two are swallowed.
    unsafe extern "system" fn keys(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            let hwnd = OVERLAY.load(SeqCst);
            if code >= 0 && hwnd != 0 {
                let kb = &*(l.0 as *const KBDLLHOOKSTRUCT);
                let msg = match kb.vkCode {
                    v if v == VK_ESCAPE.0 as u32 => Some(WM_CANCEL),
                    v if v == VK_RETURN.0 as u32 => Some(WM_SCREEN),
                    _ => None,
                };
                if let Some(m) = msg {
                    let down = matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                    if down {
                        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), m, WPARAM(0), LPARAM(0));
                    }
                    if down || matches!(w.0 as u32, WM_KEYUP | WM_SYSKEYUP) {
                        return LRESULT(1);
                    }
                }
            }
            CallNextHookEx(None::<HHOOK>, code, w, l)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Frame;
    use std::path::PathBuf;
    pub fn pictures() -> Option<PathBuf> {
        None
    }
    pub fn grab(_full: bool) -> Result<Option<Frame>, String> {
        Err("screenshots are Windows only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 12, 14, 32, 7).unwrap();
        assert_eq!(file_name(t), "OA 2026-10-12 14.32.07.png");
        let dir = std::env::temp_dir().join(format!("oa-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = free_path(&dir, "OA x.png");
        assert_eq!(a, dir.join("OA x.png"));
        std::fs::write(&a, b"").unwrap();
        assert_eq!(free_path(&dir, "OA x.png"), dir.join("OA x (2).png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crops_rows() {
        // 3×2 image, one byte per channel = pixel index.
        let px: Vec<u8> = (0..6u8).flat_map(|i| [i; 4]).collect();
        assert_eq!(crop(&px, 3, 1, 0, 3, 2), [[1u8; 4], [2; 4], [4; 4], [5; 4]].concat());
    }
}
