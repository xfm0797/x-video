//! mpv 内核的探测与安装。
//!
//! 优先使用已有的 mpv（环境变量 / 自定义路径 / 应用内安装 / 系统安装 / PATH），
//! Windows 下支持从 GitHub 一键下载解压（7z），其它平台给出安装指引。

use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// GitHub 上的 Windows 构建（静态链接，单个 exe 即可用）
const MPV_REPO: &str = "zhongfly/mpv-winbuild";
const USER_AGENT: &str = "x-video/0.4 (+https://github.com/xfm0797/x-video)";

#[derive(Clone, Debug, serde::Serialize)]
pub struct MpvBinary {
    pub path: String,
    pub version: String,
    /// 来源说明：环境变量 / 自定义路径 / 内置安装 / 系统安装 / PATH
    pub source: String,
}

pub fn install_dir() -> PathBuf {
    xv_core::store::data_dir().join("mpv")
}

pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "mpv.exe"
    } else {
        "mpv"
    }
}

fn installed_bin() -> PathBuf {
    install_dir().join(bin_name())
}

fn custom_path_file() -> PathBuf {
    install_dir().join("path.txt")
}

/// 用户在设置页填写的 mpv 路径
pub fn set_custom_path(path: &str) -> Result<()> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        let _ = std::fs::remove_file(custom_path_file());
        return Ok(());
    }
    std::fs::write(custom_path_file(), trimmed)?;
    Ok(())
}

pub fn custom_path() -> Option<PathBuf> {
    let text = std::fs::read_to_string(custom_path_file()).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// 执行 `mpv --version`，成功则返回首行（形如 `mpv 0.40.0`）
pub fn version_of(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let out = Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first.to_ascii_lowercase().starts_with("mpv") {
        Some(first)
    } else {
        None
    }
}

fn platform_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            out.push(Path::new(&pf).join("mpv").join("mpv.exe"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
            out.push(Path::new(&pf).join("mpv").join("mpv.exe"));
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            out.push(Path::new(&la).join("Programs").join("mpv").join("mpv.exe"));
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            out.push(
                Path::new(&home)
                    .join("scoop")
                    .join("apps")
                    .join("mpv")
                    .join("current")
                    .join("mpv.exe"),
            );
        }
        out.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin\mpv.exe"));
    }
    #[cfg(target_os = "macos")]
    {
        for p in [
            "/opt/homebrew/bin/mpv",
            "/usr/local/bin/mpv",
            "/Applications/mpv.app/Contents/MacOS/mpv",
            "/Applications/IINA.app/Contents/MacOS/iina-cli",
        ] {
            out.push(PathBuf::from(p));
        }
        if let Ok(home) = std::env::var("HOME") {
            out.push(Path::new(&home).join("Applications/mpv.app/Contents/MacOS/mpv"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for p in [
            "/usr/bin/mpv",
            "/usr/local/bin/mpv",
            "/snap/bin/mpv",
            "/var/lib/flatpak/exports/bin/io.mpv.Mpv",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    out
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 依次探测可用内核，返回第一个能正常 `--version` 的
pub fn detect() -> Option<MpvBinary> {
    let mut candidates: Vec<(PathBuf, &str)> = Vec::new();
    if let Ok(p) = std::env::var("XVIDEO_MPV") {
        if !p.trim().is_empty() {
            candidates.push((PathBuf::from(p.trim()), "环境变量 XVIDEO_MPV"));
        }
    }
    if let Some(p) = custom_path() {
        candidates.push((p, "自定义路径"));
    }
    candidates.push((installed_bin(), "应用内安装"));
    for p in platform_candidates() {
        candidates.push((p, "系统安装"));
    }
    if let Some(p) = which(bin_name()) {
        candidates.push((p, "PATH"));
    }

    for (path, source) in candidates {
        if let Some(version) = version_of(&path) {
            return Some(MpvBinary {
                path: path.display().to_string(),
                version,
                source: source.to_string(),
            });
        }
    }
    None
}

/// 一键安装（目前仅 Windows 可用）
pub fn download_and_install(progress: &dyn Fn(u64)) -> Result<MpvBinary> {
    if !cfg!(windows) {
        return Err(anyhow!(
            "当前平台不支持一键下载：请先安装 mpv（macOS 用 brew install mpv，Linux 用 sudo apt install mpv），然后点「重新检测」或直接填写 mpv 路径"
        ));
    }

    std::fs::create_dir_all(install_dir())?;
    let url = match std::env::var("XVIDEO_MPV_URL") {
        Ok(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => latest_asset_url()?,
    };

    progress(1);
    let archive = install_dir().join("mpv-download.7z");
    download(&url, &archive, progress).context("下载 mpv 失败")?;

    progress(85);
    let dest = install_dir().join("bin");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest)?;
    extract(&archive, &dest)?;
    let _ = std::fs::remove_file(&archive);

    let exe = find_exe(&dest, 0).ok_or_else(|| anyhow!("解压完成但未找到 {}", bin_name()))?;
    let version = version_of(&exe)
        .ok_or_else(|| anyhow!("{} 无法运行，可能缺少运行库", exe.display()))?;

    progress(100);
    Ok(MpvBinary {
        path: exe.display().to_string(),
        version,
        source: "应用内安装".to_string(),
    })
}

fn latest_asset_url() -> Result<String> {
    let api = match std::env::var("XVIDEO_MPV_INDEX") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => format!("https://api.github.com/repos/{MPV_REPO}/releases/latest"),
    };
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .build()?;
    let resp = client
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .with_context(|| format!("访问 {api} 失败（可设置 XVIDEO_MPV_URL 直接指定下载地址）"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("获取版本信息失败: HTTP {}", resp.status()));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.text().context("读取发布信息失败")?)?;
    let assets = v["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("发布信息缺少 assets 字段"))?;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        _ => "x86_64",
    };

    for a in assets {
        let name = a["name"].as_str().unwrap_or("");
        if !name.starts_with("mpv-") || !name.ends_with(".7z") {
            continue;
        }
        if name.contains("dev") || name.contains("debug") || name.contains("lgpl") {
            continue;
        }
        if !name.contains(arch) || name.contains("-v3") {
            continue;
        }
        if let Some(u) = a["browser_download_url"].as_str() {
            return Ok(u.to_string());
        }
    }
    Err(anyhow!("未在最新发布中找到 {arch} 架构的构建"))
}

fn download(url: &str, dest: &Path, progress: &dyn Fn(u64)) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(900))
        .build()?;
    let mut resp = client.get(url).send()?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last_report: u64 = 0;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        // 5% ~ 85% 之间按比例推进
        if total > 0 && done - last_report > total / 40 {
            last_report = done;
            progress(5 + (done * 80 / total).min(80));
        }
    }
    file.flush()?;
    Ok(())
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    match sevenz_rust::decompress_file(archive, dest) {
        Ok(_) => Ok(()),
        Err(e) => {
            // 回退：系统 bsdtar（Windows 10+ 自带 tar.exe 亦可解 7z）
            let out = Command::new("tar")
                .arg("-xf")
                .arg(archive)
                .arg("-C")
                .arg(dest)
                .output();
            match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(anyhow!(
                    "解压失败：7z={e}；tar={}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e2) => Err(anyhow!("解压失败：7z={e}；tar 不可用：{e2}")),
            }
        }
    }
}

fn find_exe(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth > 3 {
        return None;
    }
    let direct = dir.join(bin_name());
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe(&path, depth + 1) {
                return Some(found);
            }
        } else if path.is_file() && path.file_name().map(|n| n == bin_name()).unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_dir_under_data_dir() {
        assert!(install_dir().ends_with("mpv"));
    }

    #[test]
    fn missing_binary_has_no_version() {
        assert!(version_of(Path::new("/definitely/not/here/mpv")).is_none());
    }
}
