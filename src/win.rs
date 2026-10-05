//! Thin Win32 layer. Non-Windows stubs exist only so the crate type-checks elsewhere.

/// Physical-pixel window rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[cfg(windows)]
mod imp {
    use super::Rect;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    use windows::Win32::System::Threading::{AttachThreadInput, CreateMutexW, GetCurrentThreadId};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, INPUT, SetFocus, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, GetWindowLongPtrW, GetWindowRect, HWND_NOTOPMOST, HWND_TOPMOST,
        SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    use windows::core::w;

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut _)
    }

    pub struct Instance(HANDLE);
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn single_instance() -> Option<Instance> {
        unsafe {
            let h = CreateMutexW(None, true, w!("Local\\Omniaware.SingleInstance")).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                return None;
            }
            Some(Instance(h))
        }
    }

    pub fn foreground() -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }

    pub fn key_down(vk: i32) -> bool {
        unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 }
    }

    pub fn window_rect(h: isize) -> Option<Rect> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd(h), &mut r).ok()? };
        Some(Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top })
    }

    /// Hide → set taskbar presence → position/size/z-order → show → foreground.
    /// `rect: None` centres a `size` (logical px) on the work area of the monitor under the cursor.
    pub fn place(h: isize, rect: Option<Rect>, size: (f32, f32), topmost: bool, taskbar: bool) {
        if h == 0 {
            return;
        }
        unsafe {
            let wnd = hwnd(h);
            let _ = ShowWindow(wnd, SW_HIDE);

            let mut ex = GetWindowLongPtrW(wnd, GWL_EXSTYLE);
            let (app, tool) = (WS_EX_APPWINDOW.0 as isize, WS_EX_TOOLWINDOW.0 as isize);
            ex = if taskbar { (ex & !tool) | app } else { (ex & !app) | tool };
            SetWindowLongPtrW(wnd, GWL_EXSTYLE, ex);

            let r = rect.unwrap_or_else(|| {
                let scale = GetDpiForWindow(wnd).max(96) as f32 / 96.0;
                let (w, hh) = ((size.0 * scale) as i32, (size.1 * scale) as i32);
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let wa = if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    mi.rcWork
                } else {
                    RECT { left: 0, top: 0, right: 1920, bottom: 1080 }
                };
                Rect {
                    x: wa.left + ((wa.right - wa.left) - w).max(0) / 2,
                    y: wa.top + ((wa.bottom - wa.top) - hh).max(0) / 3,
                    w,
                    h: hh,
                }
            });
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(wnd, Some(z), r.x, r.y, r.w, r.h, SWP_FRAMECHANGED);
            let _ = ShowWindow(wnd, SW_SHOW);
        }
        activate(h);
    }

    /// Foreground + keyboard focus. Borrows the input queue of the current foreground thread
    /// instead of faking an Alt tap (winit's fallback), which can leave the previous app in
    /// menu mode where every key press dings. Returns whether we are in front now.
    pub fn activate(h: isize) -> bool {
        if h == 0 {
            return false;
        }
        unsafe {
            let wnd = hwnd(h);
            let fg = GetForegroundWindow();
            if fg == wnd {
                return true;
            }
            let me = GetCurrentThreadId();
            let other = if fg.is_invalid() { 0 } else { GetWindowThreadProcessId(fg, None) };
            let attached = other != 0 && other != me && AttachThreadInput(me, other, true).as_bool();
            let _ = BringWindowToTop(wnd);
            let _ = SetForegroundWindow(wnd);
            let _ = SetFocus(Some(wnd));
            if attached {
                let _ = AttachThreadInput(me, other, false);
            }
            GetForegroundWindow() == wnd
        }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// Paste `text` into window `target` via the clipboard (restored afterwards). Runs on a thread.
    pub fn paste_to(target: isize, text: String) {
        std::thread::spawn(move || {
            // Wait until the user lets go of modifiers (Shift+Enter would otherwise become Ctrl+Shift+V).
            for _ in 0..75 {
                let held = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
                    .iter()
                    .any(|k| key_down(k.0 as i32));
                if !held {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let Ok(mut cb) = arboard::Clipboard::new() else { return };
            let old = cb.get_text().ok();
            if cb.set_text(text).is_err() {
                return;
            }
            unsafe {
                if target != 0 {
                    let _ = SetForegroundWindow(hwnd(target));
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
                let seq = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
                SendInput(&seq, size_of::<INPUT>() as i32);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Some(old) = old {
                let _ = cb.set_text(old);
            }
        });
    }

    // ---------- Ctrl+C+C: two Ctrl-held clipboard updates in quick succession ----------

    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, RegisterClassW,
        TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
    };

    /// Updates closer than this come from a single copy (apps often write several formats).
    const SAME_COPY: Duration = Duration::from_millis(40);
    /// Max gap between the two C presses.
    const DOUBLE: Duration = Duration::from_millis(450);

    static ON_DOUBLE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);

    unsafe extern "system" fn clip_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg != WM_CLIPBOARDUPDATE {
            return unsafe { DefWindowProcW(h, msg, w, l) };
        }
        let now = Instant::now();
        let ctrl = key_down(VK_CONTROL.0 as i32);
        let fire = {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            match (*last, ctrl) {
                (_, false) => {
                    *last = None;
                    false
                }
                (Some(t), true) if now.duration_since(t) < SAME_COPY => false,
                (Some(t), true) if now.duration_since(t) <= DOUBLE => {
                    *last = None;
                    true
                }
                (_, true) => {
                    *last = Some(now);
                    false
                }
            }
        };
        if fire && let Some(f) = ON_DOUBLE.get() {
            f();
        }
        LRESULT(0)
    }

    /// Calls `f` (on a listener thread) when the user copies twice quickly with Ctrl held.
    /// Uses the clipboard-change notification, not a keyboard hook.
    pub fn on_double_copy(f: impl Fn() + Send + Sync + 'static) {
        if ON_DOUBLE.set(Box::new(f)).is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return };
            let inst: HINSTANCE = module.into();
            let class = w!("OmniawareClipboardListener");
            let wc = WNDCLASSW { lpfnWndProc: Some(clip_proc), hInstance: inst, lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(inst),
                None,
            ) {
                Ok(h) => h,
                Err(e) => return crate::log::error(format!("urklippslyssnare: {e}")),
            };
            if let Err(e) = AddClipboardFormatListener(hwnd) {
                return crate::log::error(format!("urklippslyssnare: {e}"));
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Rect;
    pub struct Instance;
    pub fn single_instance() -> Option<Instance> {
        Some(Instance)
    }
    pub fn foreground() -> isize {
        0
    }
    pub fn key_down(_vk: i32) -> bool {
        false
    }
    pub fn window_rect(_h: isize) -> Option<Rect> {
        None
    }
    pub fn place(_h: isize, _rect: Option<Rect>, _size: (f32, f32), _topmost: bool, _taskbar: bool) {}
    pub fn activate(_h: isize) -> bool {
        true
    }
    pub fn paste_to(_target: isize, _text: String) {}
    pub fn on_double_copy(_f: impl Fn() + Send + Sync + 'static) {}
}

pub use imp::*;

pub const VK_V: i32 = 0x56;

/// HWND as isize (0 if unavailable).
pub fn hwnd_of(h: &impl raw_window_handle::HasWindowHandle) -> isize {
    match h.window_handle().map(|w| w.as_raw()) {
        Ok(raw_window_handle::RawWindowHandle::Win32(w)) => w.hwnd.get(),
        _ => 0,
    }
}
