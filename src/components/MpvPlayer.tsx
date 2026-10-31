import { useCallback, useEffect, useRef, useState } from "react";
import {
  formatTime,
  loadMode,
  measureRect,
  mpv,
  saveMode,
  type MpvMode,
  type MpvStatus,
} from "../lib/mpv";

interface Props {
  url: string;
  title: string;
  onClose: () => void;
  onSwitchWeb?: () => void;
}

/**
 * mpv 内核播放器。
 *
 * 画面上屏交给原生窗口（Windows 走 `--wid` 真嵌入），HTML 只负责控制条，
 * 所以控制条刻意放在视频区域下方，避免被原生窗口盖住。
 */
export default function MpvPlayer({ url, title, onClose, onSwitchWeb }: Props) {
  const surfRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<MpvStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [opened, setOpened] = useState(false);
  const [mode, setMode] = useState<MpvMode>(loadMode());
  const [dragging, setDragging] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [pathInput, setPathInput] = useState("");

  const available = status?.available ?? false;

  // 状态轮询
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const s = await mpv.status();
        if (alive) setStatus(s);
      } catch {
        /* 服务尚未就绪 */
      }
    };
    tick();
    const timer = window.setInterval(tick, 400);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  // 打开地址
  useEffect(() => {
    if (!available) return;
    const el = surfRef.current;
    if (!el) return;
    let cancelled = false;
    setError(null);
    mpv
      .open({ url, title, mode, rect: measureRect(el) })
      .then(() => {
        if (!cancelled) setOpened(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setOpened(false);
        setError((e as Error).message);
      });
    return () => {
      cancelled = true;
    };
  }, [url, title, mode, available]);

  // 区域变化同步给原生窗口（含窗口拖动带来的屏幕坐标变化）
  useEffect(() => {
    const el = surfRef.current;
    if (!el || !opened) return;
    let timer = 0;
    let last = "";
    const push = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const target = surfRef.current;
        if (!target) return;
        const rect = measureRect(target);
        const key = `${rect.x},${rect.y},${rect.w},${rect.h},${rect.sx},${rect.sy}`;
        if (key === last) return;
        last = key;
        mpv.rect(rect).catch(() => {});
      }, 60);
    };
    const ro = new ResizeObserver(push);
    ro.observe(el);
    window.addEventListener("resize", push);
    window.addEventListener("focus", push);
    document.addEventListener("fullscreenchange", push);
    const timer2 = window.setInterval(push, 700);
    push();
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", push);
      window.removeEventListener("focus", push);
      document.removeEventListener("fullscreenchange", push);
      window.clearInterval(timer2);
      window.clearTimeout(timer);
    };
  }, [opened]);

  // 关闭播放器时收掉内核进程
  useEffect(() => {
    return () => {
      mpv.close().catch(() => {});
    };
  }, []);

  const cmd = useCallback((args: unknown[]) => {
    mpv.cmd(args).catch((e) => setError((e as Error).message));
  }, []);

  const st = status;
  const paused = st?.paused ?? true;
  const position = dragging ?? st?.position ?? 0;
  const duration = st?.duration ?? 0;

  const toggle = () => cmd(["set_property", "pause", !paused]);
  const seekBy = (delta: number) => cmd(["seek", delta, "relative"]);
  const commitSeek = () => {
    if (dragging !== null) {
      cmd(["seek", dragging, "absolute"]);
      setDragging(null);
    }
  };
  const toggleMute = () => cmd(["set_property", "mute", !(st?.mute ?? false)]);
  const setVolume = (v: number) => cmd(["set_property", "volume", v]);

  const toggleFull = async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch {
      /* 全屏不可用时忽略 */
    }
  };

  const switchMode = () => {
    const next: MpvMode = mode === "embed" ? "floating" : "embed";
    saveMode(next);
    setMode(next);
    setOpened(false);
  };

  const doInstall = async () => {
    setBusy(true);
    setError(null);
    try {
      await mpv.install();
      const s = await mpv.probe();
      setStatus(s);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const doProbe = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await mpv.probe());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const savePath = async () => {
    if (!pathInput.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setStatus(await mpv.setPath(pathInput.trim()));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
        else onClose();
        return;
      }
      if (!opened) return;
      if (e.code === "Space") {
        e.preventDefault();
        toggle();
      } else if (e.code === "ArrowLeft") seekBy(-10);
      else if (e.code === "ArrowRight") seekBy(10);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opened, paused, onClose]);

  const showSetup = status !== null && !available;

  return (
    <div className="player-overlay">
      <div className="player-head">
        <button className="btn ghost" onClick={onClose}>
          ← 返回
        </button>
        <span className="player-title">{title}</span>
        <span className="badge kind-other">mpv</span>
        {onSwitchWeb && (
          <button className="btn ghost" onClick={onSwitchWeb}>
            浏览器内核
          </button>
        )}
        <button className="btn ghost" onClick={switchMode} title="嵌入模式不生效时可切浮动窗口">
          {mode === "embed" ? "浮动窗口" : "嵌入窗口"}
        </button>
      </div>

      {(error || status?.error) && (
        <div className="mpv-banner">{error || status?.error}</div>
      )}

      {status === null && <div className="player-loading">检测播放内核…</div>}

      {showSetup && (
        <div className="mpv-setup">
          <h3>未检测到 mpv 内核</h3>
          <p className="hint">
            FLV / RMVB / MKV / DASH 等格式需要 mpv 内核（libmpv）。安装后所有此前无法播放的源都可用。
          </p>
          {status?.installing || busy ? (
            <div className="mpv-progress">
              <div className="bar" style={{ width: `${Math.max(3, status?.progress ?? 0)}%` }} />
            </div>
          ) : null}
          <div className="mpv-setup-actions">
            {status?.can_install && !status?.installing && (
              <button className="btn" onClick={doInstall} disabled={busy}>
                {busy ? "处理中…" : "一键下载安装（约 30MB）"}
              </button>
            )}
            <button className="btn ghost" onClick={doProbe} disabled={busy}>
              重新检测
            </button>
          </div>
          <div className="mpv-setup-path">
            <input
              placeholder={
                status?.platform === "windows"
                  ? "或填写 mpv.exe 路径"
                  : "或填写 mpv 路径（如 /opt/homebrew/bin/mpv）"
              }
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
            />
            <button className="btn ghost" onClick={savePath} disabled={busy}>
              使用该路径
            </button>
          </div>
          {!status?.can_install && (
            <p className="hint">
              macOS：<code>brew install mpv</code>；Linux：<code>sudo apt install mpv</code>，装完点「重新检测」。
            </p>
          )}
          <p className="hint dim">安装目录：{status?.install_dir}</p>
        </div>
      )}

      {available && (
        <div className="player-stage mpv-stage">
          <div className="mpv-surface" ref={surfRef}>
            {!opened && <div className="player-loading">启动内核中…</div>}
          </div>
          <div className="mpv-controls">
            <button className="mpv-btn" onClick={toggle} title="播放/暂停（空格）">
              {paused ? "▶" : "❚❚"}
            </button>
            <button className="mpv-btn" onClick={() => seekBy(-10)} title="后退 10s">
              ⟲10
            </button>
            <button className="mpv-btn" onClick={() => seekBy(10)} title="前进 10s">
              10⟳
            </button>
            <span className="mpv-time">{formatTime(position)}</span>
            <input
              className="mpv-seek"
              type="range"
              min={0}
              max={duration > 0 ? duration : 0}
              step={0.5}
              value={Math.min(position, duration > 0 ? duration : position)}
              onChange={(e) => setDragging(Number(e.target.value))}
              onMouseUp={commitSeek}
              onTouchEnd={commitSeek}
              onKeyUp={commitSeek}
              disabled={duration <= 0}
            />
            <span className="mpv-time">{formatTime(duration)}</span>
            <button className="mpv-btn" onClick={toggleMute} title="静音">
              {st?.mute ? "🔇" : "🔊"}
            </button>
            <input
              className="mpv-vol"
              type="range"
              min={0}
              max={150}
              step={1}
              value={st?.volume ?? 100}
              onChange={(e) => setVolume(Number(e.target.value))}
            />
            <button className="mpv-btn" onClick={toggleFull} title="全屏">
              ⛶
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
