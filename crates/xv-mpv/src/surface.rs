//! 视频输出承载面（Surface）。
//!
//! - Windows：自建原生窗口，把 `--wid` 交给 mpv，mpv 在其中渲染子窗口。
//!   `Embed` 模式挂成主窗口的子窗口（看起来就是页面里的一块区域）；
//!   `Floating` 模式是主窗口的 owned popup（不进任务栏、始终在主窗口之上）。
//! - macOS / Linux：不做原生嵌入，改用无边框置顶窗口 + `--geometry` 定位，
//!   位置由前端上报的屏幕坐标决定。

use anyhow::Result;
use serde::{Deserialize, Serialize};

fn one() -> f64 {
    1.0
}

/// 前端上报的显示区域（x/y/w/h 为物理像素，相对窗口客户区；
/// sx/sy 为物理像素的屏幕坐标，供非 Windows 的浮动窗口使用）
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub sx: f64,
    #[serde(default)]
    pub sy: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 960.0,
            h: 540.0,
            scale: 1.0,
            sx: 0.0,
            sy: 0.0,
        }
    }
}

impl Rect {
    /// 交给 mpv --geometry 的 CSS 像素版本（左上角 + 宽高）
    fn geometry(&self) -> String {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let w = (self.w / scale).round().max(16.0) as i64;
        let h = (self.h / scale).round().max(16.0) as i64;
        let x = (self.sx / scale).round() as i64;
        let y = (self.sy / scale).round() as i64;
        format!("{w}x{h}+{x}+{y}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceMode {
    /// 嵌入主窗口（默认）
    #[default]
    Embed,
    /// 独立浮动窗口（嵌入不生效时的兜底）
    Floating,
}

pub struct Surface {
    pub mode: SurfaceMode,
    rect: Rect,
    #[cfg(windows)]
    hwnd: isize,
}

pub fn create(mode: SurfaceMode, rect: Rect) -> Result<Surface> {
    #[cfg(windows)]
    {
        win::create(mode, rect)
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 平台一律走浮动窗口
        let _ = mode;
        Ok(Surface {
            mode: SurfaceMode::Floating,
            rect,
        })
    }
}

impl Surface {
    pub fn resize(&mut self, rect: Rect) -> Result<()> {
        self.rect = rect;
        #[cfg(windows)]
        win::resize(self.hwnd, self.mode, rect)?;
        Ok(())
    }

    /// mpv 启动参数里的 `--wid=`
    pub fn wid_arg(&self) -> Option<String> {
        #[cfg(windows)]
        {
            if self.hwnd != 0 {
                return Some(format!("--wid={}", self.hwnd));
            }
        }
        None
    }

    /// 非 Windows 浮动窗口需要的额外参数
    pub fn extra_args(&self) -> Vec<String> {
        if self.wid_arg().is_some() {
            return Vec::new();
        }
        vec![
            "--no-border".to_string(),
            "--ontop".to_string(),
            format!("--geometry={}", self.rect.geometry()),
        ]
    }

    pub fn destroy(self) {
        #[cfg(windows)]
        win::destroy(self.hwnd);
    }
}

#[cfg(windows)]
mod win {
    //! 极简 Win32 绑定：只用到创建/移动/销毁窗口与枚举自身窗口，
    //! 自己声明比引入 windows crate 更稳定（不受其版本 API 变动影响）。

    use super::{Rect, Surface, SurfaceMode};
    use anyhow::{anyhow, Result};
    use std::ffi::c_void;

    type Hwnd = isize;

    const WS_CHILD: u32 = 0x4000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_CLIPCHILDREN: u32 = 0x0200_0000;
    const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    const WS_POPUP: u32 = 0x8000_0000;
    const SS_BLACKRECT: u32 = 0x0000_0004;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const GW_OWNER: u32 = 4;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const HWND_TOP: Hwnd = 0;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindow(hwnd: Hwnd, cmd: u32) -> Hwnd;
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: isize,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> Hwnd;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn SetWindowPos(
            hwnd: Hwnd,
            after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn IsWindow(hwnd: Hwnd) -> i32;
        fn GetClientRect(hwnd: Hwnd, rect: *mut Rect4) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    #[repr(C)]
    struct Rect4 {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    thread_local! {
        static BEST: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
        static BEST_AREA: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }

    extern "system" fn enum_cb(hwnd: Hwnd, _param: isize) -> i32 {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != GetCurrentProcessId() || IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            // 只要无 owner 的顶层窗口（Tauri 主窗口）
            if GetWindow(hwnd, GW_OWNER) != 0 {
                return 1;
            }
            let mut rc = Rect4 {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClientRect(hwnd, &mut rc) == 0 {
                return 1;
            }
            let area = (rc.right - rc.left) as i64 * (rc.bottom - rc.top) as i64;
            if area > BEST_AREA.with(|c| c.get()) {
                BEST_AREA.with(|c| c.set(area));
                BEST.with(|c| c.set(hwnd));
            }
        }
        1
    }

    /// 找到本进程的主窗口句柄
    fn parent_window() -> Option<Hwnd> {
        if let Ok(v) = std::env::var("XVIDEO_MPV_PARENT_HWND") {
            if let Ok(n) = v.trim().parse::<isize>() {
                if n != 0 {
                    return Some(n);
                }
            }
        }
        BEST.with(|c| c.set(0));
        BEST_AREA.with(|c| c.set(0));
        unsafe {
            EnumWindows(enum_cb, 0);
        }
        let hwnd = BEST.with(|c| c.get());
        if hwnd != 0 {
            Some(hwnd)
        } else {
            None
        }
    }

    pub fn create(mode: SurfaceMode, rect: Rect) -> Result<Surface> {
        let parent = parent_window();
        let mut mode = mode;
        if mode == SurfaceMode::Embed && parent.is_none() {
            eprintln!("未找到主窗口句柄，mpv 改为浮动窗口模式");
            mode = SurfaceMode::Floating;
        }
        let parent = parent.unwrap_or(0);

        let (style, ex_style, x, y) = match mode {
            SurfaceMode::Embed => (
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT,
                0u32,
                rect.x.round() as i32,
                rect.y.round() as i32,
            ),
            SurfaceMode::Floating => (
                WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | SS_BLACKRECT,
                WS_EX_TOOLWINDOW,
                rect.sx.round() as i32,
                rect.sy.round() as i32,
            ),
        };

        let class = wide("STATIC");
        let name = wide("");
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class.as_ptr(),
                name.as_ptr(),
                style,
                x,
                y,
                rect.w.round().max(32.0) as i32,
                rect.h.round().max(32.0) as i32,
                parent,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if hwnd == 0 {
            return Err(anyhow!("创建 mpv 输出窗口失败（--wid 无法建立）"));
        }
        // 保证盖在同级 WebView 子窗口之上
        let _ = resize(hwnd, mode, rect);
        Ok(Surface {
            mode,
            rect,
            hwnd,
        })
    }

    pub fn resize(hwnd: Hwnd, mode: SurfaceMode, rect: Rect) -> Result<()> {
        if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
            return Err(anyhow!("mpv 输出窗口已失效"));
        }
        let (x, y) = match mode {
            SurfaceMode::Embed => (rect.x.round() as i32, rect.y.round() as i32),
            SurfaceMode::Floating => (rect.sx.round() as i32, rect.sy.round() as i32),
        };
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                x,
                y,
                rect.w.round().max(32.0) as i32,
                rect.h.round().max(32.0) as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        Ok(())
    }

    pub fn destroy(hwnd: Hwnd) {
        if hwnd != 0 && unsafe { IsWindow(hwnd) } != 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
        }
    }
}
