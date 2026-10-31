import { useEffect, useState } from "react";
import { loadMode, mpv, saveMode, type MpvMode, type MpvStatus } from "../lib/mpv";

export default function Settings() {
  const [status, setStatus] = useState<MpvStatus | null>(null);
  const [mode, setMode] = useState<MpvMode>(loadMode());
  const [pathInput, setPathInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ kind: "ok" | "err"; text: string } | null>(
    null
  );

  const refresh = async () => {
    try {
      setStatus(await mpv.status());
    } catch (e) {
      setMsg({ kind: "err", text: (e as Error).message });
    }
  };

  useEffect(() => {
    refresh();
    const timer = window.setInterval(() => {
      mpv.status().then(setStatus).catch(() => {});
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const run = async (fn: () => Promise<MpvStatus | { path: string; version: string }>, ok: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await fn();
      setStatus(await mpv.status());
      setMsg({ kind: "ok", text: ok });
    } catch (e) {
      setMsg({ kind: "err", text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const pickMode = (m: MpvMode) => {
    setMode(m);
    saveMode(m);
  };

  return (
    <div className="page">
      <h2>设置</h2>

      <div className="card">
        <h3>播放内核</h3>
        <p className="hint">
          浏览器内核（hls.js / video）负责 HLS、MP4；mpv 内核（libmpv）负责
          FLV、MKV、AVI、RMVB、DASH 等容器，也是各类 fallback 的兜底。
        </p>
        <div className="kv">
          <span>状态</span>
          <b className={status === null ? "" : status.available ? "ok" : "err"}>
            {status === null
              ? "检测中…"
              : status.available
                ? "已就绪"
                : "未检测到"}
          </b>
        </div>
        {status?.available && (
          <>
            <div className="kv">
              <span>版本</span>
              <b>{status.version}</b>
            </div>
            <div className="kv">
              <span>路径</span>
              <b className="mono">{status.path}</b>
            </div>
            <div className="kv">
              <span>来源</span>
              <b>{status.source}</b>
            </div>
          </>
        )}
        <div className="kv">
          <span>安装目录</span>
          <b className="mono">{status?.install_dir || "-"}</b>
        </div>

        <div className="row-actions">
          <button className="btn ghost" onClick={refresh} disabled={busy}>
            重新检测
          </button>
          {status?.can_install && (
            <button
              className="btn"
              disabled={busy || status.installing}
              onClick={() =>
                run(() => mpv.install(), "mpv 内核安装完成，可以直接播放 FLV/RMVB 了")
              }
            >
              {status.installing ? `下载中 ${status.progress}%` : "一键下载安装 mpv"}
            </button>
          )}
        </div>
        {status?.installing && (
          <div className="mpv-progress">
            <div className="bar" style={{ width: `${Math.max(3, status.progress)}%` }} />
          </div>
        )}

        <div className="mpv-setup-path">
          <input
            placeholder="指定已有 mpv 可执行文件路径"
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
          />
          <button
            className="btn ghost"
            disabled={busy || !pathInput.trim()}
            onClick={() =>
              run(() => mpv.setPath(pathInput.trim()), "已应用自定义 mpv 路径")
            }
          >
            应用路径
          </button>
        </div>

        {!status?.can_install && (
          <p className="hint">
            macOS 可用 <code>brew install mpv</code>，Linux 可用{" "}
            <code>sudo apt install mpv</code>，装完点「重新检测」。
          </p>
        )}
        {msg && <div className={`msg ${msg.kind}`}>{msg.text}</div>}
      </div>

      <div className="card">
        <h3>画面承载方式</h3>
        <p className="hint">
          mpv 的画面由原生窗口绘制，默认嵌进应用窗口；若出现黑块、位置偏移（少数
          WebView2 / 窗口管理器环境），可切换为浮动窗口。
        </p>
        <div className="row-actions">
          <button
            className={`btn ${mode === "embed" ? "" : "ghost"}`}
            onClick={() => pickMode("embed")}
          >
            嵌入窗口
          </button>
          <button
            className={`btn ${mode === "floating" ? "" : "ghost"}`}
            onClick={() => pickMode("floating")}
          >
            浮动窗口
          </button>
        </div>
      </div>

      <div className="card">
        <h3>关于</h3>
        <p className="hint">X-Video v0.4.1 · Tauri 2 + React + Rust</p>
        <p className="hint">
          本地服务地址：http://127.0.0.1:21600（API、流媒体代理与内核控制）
        </p>
      </div>

      <div className="card">
        <h3>路线图</h3>
        <ul className="roadmap">
          <li>✅ 源管理（M3U / TXT / TVBox 配置）</li>
          <li>✅ IPTV 直播播放（含 m3u8 代理重写）</li>
          <li>✅ TVBox type=1 JSON CMS 点播（分类 / 列表 / 详情 / 播放 / 搜索）</li>
          <li>✅ jx 网页解析（站点 playUrl 模板 / parses 接口 / 页面直链抽取）</li>
          <li>✅ JS Spider 沙箱（Boa 引擎：req / pdfh / pdfa / base64 / md5 / local）</li>
          <li>✅ 聚合搜索（并发多站点，结果标注来源）</li>
          <li>✅ libmpv 播放内核（FLV / MKV / AVI / RMVB / DASH，双内核自动切换）</li>
          <li>🚧 收藏与播放历史、EPG 节目单</li>
          <li>🚧 自动更新</li>
        </ul>
      </div>
    </div>
  );
}
