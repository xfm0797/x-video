//! libmpv 的 C API 绑定：只做动态加载与最小封装，不编译期链接。
//!
//! 这样安装包里换一个 `libmpv-2.dll` 就能升级内核，CI 也不需要预装
//! libmpv 开发包（避免三平台 SDK 分发问题）。

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

use anyhow::{anyhow, Result};
use libloading::Library;

pub type Handle = c_void;

#[allow(dead_code)]
pub const FORMAT_NONE: c_int = 0;
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

type FnCreate = unsafe extern "C" fn() -> *mut Handle;
type FnInitialize = unsafe extern "C" fn(*mut Handle) -> c_int;
type FnDestroy = unsafe extern "C" fn(*mut Handle);
type FnSetOptionString = unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnGetProperty = unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnSetPropertyString = unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int;
type FnGetPropertyString = unsafe extern "C" fn(*mut Handle, *const c_char) -> *mut c_char;
type FnCommand = unsafe extern "C" fn(*mut Handle, *const *const c_char) -> c_int;
type FnObserveProperty =
    unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut Handle, f64) -> *mut MpvEvent;
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnFree = unsafe extern "C" fn(*mut c_void);

pub struct Api {
    _lib: Library,
    pub path: PathBuf,
    pub create: FnCreate,
    pub initialize: FnInitialize,
    pub terminate_destroy: FnDestroy,
    pub set_option_string: FnSetOptionString,
    pub set_property: FnSetProperty,
    pub get_property: FnGetProperty,
    pub set_property_string: FnSetPropertyString,
    pub get_property_string: FnGetPropertyString,
    pub command: FnCommand,
    pub observe_property: FnObserveProperty,
    pub wait_event: FnWaitEvent,
    pub error_string: FnErrorString,
    pub free: FnFree,
}

// libmpv 的句柄本身是线程安全的（mpv 内部有锁），函数指针在库卸载前始终有效。
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// 依次尝试：环境变量 → 可执行文件同目录 → resources/mpv → 系统目录。
pub fn locate() -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(p) = std::env::var("XVIDEO_LIBMPV") {
        if !p.trim().is_empty() {
            candidates.push(PathBuf::from(p));
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in [
                "libmpv-2.dll",
                "mpv/libmpv-2.dll",
                "resources/mpv/libmpv-2.dll",
                "../Frameworks/libmpv-2.dll",
            ] {
                candidates.push(dir.join(rel));
            }
        }
    }

    candidates.push(PathBuf::from("libmpv-2.dll"));
    candidates.push(PathBuf::from("C:/Windows/System32/libmpv-2.dll"));

    for p in &candidates {
        if p.is_file() {
            return Ok(p.clone());
        }
    }

    Err(anyhow!(
        "未找到 libmpv-2.dll（已查找 {} 个位置），请重新安装或设置 XVIDEO_LIBMPV 指向该文件",
        candidates.len()
    ))
}

impl Api {
    pub fn load(path: &Path) -> Result<Self> {
        unsafe {
            let lib = Library::new(path)
                .map_err(|e| anyhow!("加载 {} 失败：{e}", path.display()))?;

            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let s: libloading::Symbol<'_, $ty> = lib.get($name.as_bytes()).map_err(
                        |e| anyhow!("libmpv 缺少符号 {}：{e}", $name),
                    )?;
                    *s
                }};
            }

            Ok(Self {
                path: path.to_path_buf(),
                create: sym!("mpv_create\0", FnCreate),
                initialize: sym!("mpv_initialize\0", FnInitialize),
                terminate_destroy: sym!("mpv_terminate_destroy\0", FnDestroy),
                set_option_string: sym!("mpv_set_option_string\0", FnSetOptionString),
                set_property: sym!("mpv_set_property\0", FnSetProperty),
                get_property: sym!("mpv_get_property\0", FnGetProperty),
                set_property_string: sym!("mpv_set_property_string\0", FnSetPropertyString),
                get_property_string: sym!("mpv_get_property_string\0", FnGetPropertyString),
                command: sym!("mpv_command\0", FnCommand),
                observe_property: sym!("mpv_observe_property\0", FnObserveProperty),
                wait_event: sym!("mpv_wait_event\0", FnWaitEvent),
                error_string: sym!("mpv_error_string\0", FnErrorString),
                free: sym!("mpv_free\0", FnFree),
                _lib: lib,
            })
        }
    }

    pub fn error_text(&self, code: c_int) -> String {
        unsafe {
            let p = (self.error_string)(code);
            if p.is_null() {
                return format!("mpv 错误 {code}");
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    /// 库路径，便于在设置页展示。
    pub fn path(&self) -> String {
        self.path.display().to_string()
    }
}

/// 一个 mpv 实例的操作封装（命令、属性、事件读取）。
pub struct Mpv {
    pub api: &'static Api,
    pub ctx: *mut Handle,
}

unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

impl Mpv {
    pub fn new(api: &'static Api, options: &[(&str, String)]) -> Result<Self> {
        unsafe {
            let ctx = (api.create)();
            if ctx.is_null() {
                return Err(anyhow!("mpv_create 返回空指针"));
            }
            let mpv = Self { api, ctx };

            for (k, v) in options {
                mpv.option(k, v)?;
            }
            let rc = (api.initialize)(ctx);
            if rc < 0 {
                return Err(anyhow!("mpv 初始化失败：{}", api.error_text(rc)));
            }
            Ok(mpv)
        }
    }

    pub fn option(&self, name: &str, value: &str) -> Result<()> {
        let n = CString::new(name)?;
        let v = CString::new(value)?;
        let rc = unsafe { (self.api.set_option_string)(self.ctx, n.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            return Err(anyhow!(
                "设置选项 {name}={value} 失败：{}",
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        let owned: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s).unwrap_or_default())
            .collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        let rc = unsafe { (self.api.command)(self.ctx, ptrs.as_ptr()) };
        if rc < 0 {
            return Err(anyhow!(
                "命令 {} 失败：{}",
                args.join(" "),
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    pub fn set_flag(&self, name: &str, value: bool) -> Result<()> {
        let n = CString::new(name)?;
        let mut v: c_int = i32::from(value);
        let rc = unsafe {
            (self.api.set_property)(
                self.ctx,
                n.as_ptr(),
                FORMAT_FLAG,
                &mut v as *mut c_int as *mut c_void,
            )
        };
        if rc < 0 {
            return Err(anyhow!(
                "设置 {name} 失败：{}",
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    pub fn set_f64(&self, name: &str, value: f64) -> Result<()> {
        let n = CString::new(name)?;
        let mut v = value;
        let rc = unsafe {
            (self.api.set_property)(
                self.ctx,
                n.as_ptr(),
                FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            )
        };
        if rc < 0 {
            return Err(anyhow!(
                "设置 {name} 失败：{}",
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    pub fn set_string(&self, name: &str, value: &str) -> Result<()> {
        let n = CString::new(name)?;
        let v = CString::new(value)?;
        let rc = unsafe { (self.api.set_property_string)(self.ctx, n.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            return Err(anyhow!(
                "设置 {name} 失败：{}",
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = CString::new(name).ok()?;
        unsafe {
            let p = (self.api.get_property_string)(self.ctx, n.as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.free)(p as *mut c_void);
            Some(s)
        }
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let n = CString::new(name).ok()?;
        let mut v: f64 = 0.0;
        let rc = unsafe {
            (self.api.get_property)(
                self.ctx,
                n.as_ptr(),
                FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            )
        };
        (rc >= 0).then_some(v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let n = CString::new(name).ok()?;
        let mut v: i64 = 0;
        let rc = unsafe {
            (self.api.get_property)(
                self.ctx,
                n.as_ptr(),
                FORMAT_INT64,
                &mut v as *mut i64 as *mut c_void,
            )
        };
        (rc >= 0).then_some(v)
    }

    pub fn get_flag(&self, name: &str) -> Option<bool> {
        self.get_i64(name).map(|v| v != 0)
    }

    pub fn observe(&self, name: &str, format: c_int) -> Result<()> {
        let n = CString::new(name)?;
        let rc = unsafe { (self.api.observe_property)(self.ctx, 0, n.as_ptr(), format) };
        if rc < 0 {
            return Err(anyhow!(
                "订阅 {name} 失败：{}",
                self.api.error_text(rc)
            ));
        }
        Ok(())
    }

    /// 阻塞等待一个事件，超时返回 None。
    pub fn wait_event(&self, timeout: f64) -> Option<&MpvEvent> {
        unsafe {
            let p = (self.api.wait_event)(self.ctx, timeout);
            if p.is_null() {
                return None;
            }
            if (*p).event_id == EVENT_NONE {
                return None;
            }
            Some(&*p)
        }
    }

    pub fn shutdown(&self) {
        unsafe { (self.api.terminate_destroy)(self.ctx) }
    }
}
