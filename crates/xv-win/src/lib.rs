//! Windows 原生承载面：给 mpv 一个真实的子窗口句柄。
//!
//! mpv 的 `--wid` 只认 HWND，画面是原生渲染，HTML 永远盖不住它。因此播放页
//! 把视频区单独切出来：前端上报区域矩形，这里把子窗口挪过去并置顶，控制条
//! 放在视频区之外。

#![allow(non_snake_case)]

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    type HWND = isize;
    type BOOL = i32;
    type DWORD = u32;
    type HINSTANCE = isize;
    type HMENU = isize;
    type LPARAM = isize;

    const WS_CHILD: DWORD = 0x4000_0000;
    const WS_VISIBLE: DWORD = 0x1000_0000;
    const WS_CLIPCHILDREN: DWORD = 0x0200_0000;
    const WS_CLIPSIBLINGS: DWORD = 0x0400_0000;
    const WS_EX_NOACTIVATE: DWORD = 0x0800_0000;
    const WS_EX_TRANSPARENT: DWORD = 0x0000_0020;
    const SS_BLACKRECT: DWORD = 0x0000_0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SWP_HIDEWINDOW: u32 = 0x0080;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    const GWL_STYLE: i32 = -16;
    const WS_OVERLAPPEDWINDOW: isize = 0x00CF_0000;
    const WS_POPUP: isize = 0x8000_0000;
    const WS_VISIBLE_L: isize = 0x1000_0000;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const HWND_TOP: isize = 0;
    const GW_OWNER: u32 = 4;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;

    #[repr(C)]
    #[derive(Default)]
    struct MONITORINFO {
        cb_size: u32,
        rc_monitor: RECT,
        rc_work: RECT,
        flags: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn CreateWindowExW(
            ex: DWORD,
            class: *const u16,
            name: *const u16,
            style: DWORD,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: HWND,
            menu: HMENU,
            instance: HINSTANCE,
            param: *mut c_void,
        ) -> HWND;
        fn DestroyWindow(hwnd: HWND) -> BOOL;
        fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
        fn SetWindowPos(
            hwnd: HWND,
            after: HWND,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> BOOL;
        fn EnumWindows(cb: extern "system" fn(HWND, LPARAM) -> BOOL, param: LPARAM) -> BOOL;
        fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut DWORD) -> DWORD;
        fn GetWindow(hwnd: HWND, cmd: u32) -> HWND;
        fn IsWindowVisible(hwnd: HWND) -> BOOL;
        fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
        fn IsWindow(hwnd: HWND) -> BOOL;
        fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
        fn MonitorFromWindow(hwnd: HWND, flags: u32) -> isize;
        fn GetMonitorInfoW(monitor: isize, info: *mut MONITORINFO) -> BOOL;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> DWORD;
        fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct FindCtx {
        pid: DWORD,
        best: HWND,
        area: i64,
    }

    extern "system" fn enum_cb(hwnd: HWND, param: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(param as *mut FindCtx) };
        unsafe {
            let mut pid: DWORD = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != ctx.pid || IsWindowVisible(hwnd) == 0 || GetWindow(hwnd, GW_OWNER) != 0 {
                return 1;
            }
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return 1;
            }
            let area = (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64;
            if area > ctx.area {
                ctx.area = area;
                ctx.best = hwnd;
            }
        }
        1
    }

    /// 找当前进程里面积最大的顶层窗口，即 Tauri 主窗口。
    pub fn main_hwnd() -> isize {
        if let Ok(raw) = std::env::var("XVIDEO_PARENT_HWND") {
            if let Ok(v) = raw.parse::<isize>() {
                if v != 0 {
                    return v;
                }
            }
        }
        let mut ctx = FindCtx {
            pid: unsafe { GetCurrentProcessId() },
            best: 0,
            area: 0,
        };
        unsafe {
            EnumWindows(enum_cb, &mut ctx as *mut FindCtx as LPARAM);
        }
        ctx.best
    }

    pub fn create(parent: isize, x: i32, y: i32, w: i32, h: i32) -> isize {
        if parent == 0 {
            return 0;
        }
        let class = wide("STATIC");
        let title = wide("");
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT,
                x,
                y,
                w.max(1),
                h.max(1),
                parent,
                0,
                instance,
                std::ptr::null_mut(),
            );
            hwnd
        }
    }

    pub fn set_rect(hwnd: isize, x: i32, y: i32, w: i32, h: i32, visible: bool) {
        if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
            return;
        }
        let flags = SWP_NOACTIVATE | if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                x,
                y,
                w.max(1),
                h.max(1),
                flags,
            );
            ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }

    /// 主窗口全屏切换。Win32 的窗口样式由我们直接改，避免依赖 Webview 的
    /// Fullscreen API（WebView2 里那是元素级全屏，窗口不会真的铺满显示器）。
    pub fn set_fullscreen(hwnd: isize, on: bool) -> bool {
        use std::sync::Mutex;
        static SAVED: Mutex<Option<(isize, i32, i32, i32, i32)>> = Mutex::new(None);

        if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
            return false;
        }
        unsafe {
            if on {
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect);
                if let Ok(mut slot) = SAVED.lock() {
                    if slot.is_none() {
                        *slot = Some((style, rect.left, rect.top, rect.right, rect.bottom));
                    }
                }
                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cb_size: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(monitor, &mut info) == 0 {
                    return false;
                }
                SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !WS_OVERLAPPEDWINDOW) | WS_POPUP | WS_VISIBLE_L);
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    info.rc_monitor.left,
                    info.rc_monitor.top,
                    info.rc_monitor.right - info.rc_monitor.left,
                    info.rc_monitor.bottom - info.rc_monitor.top,
                    SWP_FRAMECHANGED | SWP_NOACTIVATE,
                );
                true
            } else {
                let saved = SAVED.lock().ok().and_then(|mut s| s.take());
                if let Some((style, l, t, r, b)) = saved {
                    SetWindowLongPtrW(hwnd, GWL_STYLE, style);
                    SetWindowPos(
                        hwnd,
                        HWND_TOP,
                        l,
                        t,
                        (r - l).max(800),
                        (b - t).max(600),
                        SWP_FRAMECHANGED | SWP_NOACTIVATE,
                    );
                }
                true
            }
        }
    }

    pub fn destroy(hwnd: isize) {
        if hwnd != 0 && unsafe { IsWindow(hwnd) } != 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
        }
    }
}

#[cfg(windows)]
pub use win::{
    create as create_surface, destroy as destroy_surface, main_hwnd,
    set_fullscreen, set_rect as set_surface,
};

#[cfg(not(windows))]
mod stub {
    pub fn main_hwnd() -> isize {
        0
    }
    pub fn create(_parent: isize, _x: i32, _y: i32, _w: i32, _h: i32) -> isize {
        0
    }
    pub fn set_rect(_h: isize, _x: i32, _y: i32, _w: i32, _h2: i32, _v: bool) {}
    pub fn destroy(_h: isize) {}
    pub fn set_fullscreen(_h: isize, _on: bool) -> bool {
        false
    }
}

#[cfg(not(windows))]
pub use stub::{create_surface, destroy_surface, main_hwnd, set_fullscreen, set_surface};
