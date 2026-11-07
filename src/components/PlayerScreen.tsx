import { useCallback, useEffect, useRef, useState } from "react";
import { api, art, type Item, type PlaybackState } from "../lib/api";
import { epLabel, fmtTime, progressPct } from "../lib/format";
import { Icon } from "./Icon";

type Props = {
  itemId?: number;
  url?: string;
  title: string;
  kind: "video" | "audio" | "live";
  /** 在线点播：不参与本地队列，也不显示选集抽屉 */
  remote?: boolean;
  onClose: () => void;
};

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];

export function PlayerScreen({ itemId, url, title, kind, remote: remoteProp, onClose }: Props) {
  const remote = remoteProp ?? (!!url || kind === "live");
  const live = kind === "live";
  const [st, setSt] = useState<PlaybackState | null>(null);
  const [queue, setQueue] = useState<Item[]>([]);
  const [drag, setDrag] = useState<number | null>(null);
  const [drawer, setDrawer] = useState(false);
  const [fs, setFs] = useState(false);
  const surfRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const s = await api.state();
        if (alive) setSt(s);
      } catch {
        /* 服务重启期间的瞬断忽略 */
      }
    };
    tick();
    const t = window.setInterval(tick, 260);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [itemId]);

  useEffect(() => {
    if (!itemId) {
      setQueue([]);
      return;
    }
    api
      .queue(itemId)
      .then((r) => setQueue(r.items))
      .catch(() => setQueue([]));
  }, [itemId]);

  // 原生画面跟着 DOM 区域走：窗口拖动不触发 ResizeObserver，另加轮询兜底
  useEffect(() => {
    if (kind === "audio") return;
    let alive = true;
    const push = () => {
      const el = surfRef.current;
      if (!el || !alive) return;
      const r = el.getBoundingClientRect();
      const d = window.devicePixelRatio || 1;
      api
        .rect({
          x: Math.round(r.left * d),
          y: Math.round(r.top * d),
          w: Math.round(r.width * d),
          h: Math.round(r.height * d),
          visible: true,
        })
        .catch(() => {});
    };
    const t0 = window.setTimeout(push, 50);
    const ro = new ResizeObserver(push);
    if (surfRef.current) ro.observe(surfRef.current);
    window.addEventListener("resize", push);
    const iv = window.setInterval(push, 800);
    return () => {
      alive = false;
      window.clearTimeout(t0);
      ro.disconnect();
      window.removeEventListener("resize", push);
      window.clearInterval(iv);
      api.rect({ x: 0, y: 0, w: 0, h: 0, visible: false }).catch(() => {});
    };
  }, [kind]);

  useEffect(() => () => {
    api.fullscreen(false).catch(() => {});
  }, []);

  const engine = st?.engine;
  const dur = engine?.duration ?? 0;
  const pos = drag ?? engine?.position ?? 0;
  const pct = progressPct(pos, dur);

  const togglePlay = useCallback(() => {
    api.cmd("toggle").catch(() => {});
  }, []);

  const seekBy = useCallback(
    (d: number) => {
      const p = st?.engine.position ?? 0;
      api.cmd("seek", { value: Math.max(0, p + d) }).catch(() => {});
    },
    [st?.engine.position]
  );

  const toggleFullscreen = useCallback(() => {
    setFs((v) => {
      api.fullscreen(!v).catch(() => {});
      return !v;
    });
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      let handled = true;
      switch (e.key) {
        case " ":
        case "k":
          togglePlay();
          break;
        case "ArrowLeft":
          if (live) handled = false;
          else seekBy(-10);
          break;
        case "ArrowRight":
          if (live) handled = false;
          else seekBy(10);
          break;
        case "ArrowUp":
          api.cmd("volume", { value: Math.min(150, (st?.engine.volume ?? 100) + 5) });
          break;
        case "ArrowDown":
          api.cmd("volume", { value: Math.max(0, (st?.engine.volume ?? 100) - 5) });
          break;
        case "m":
          api.cmd("mute", { value: st?.engine.muted ? 0 : 1 });
          break;
        case "f":
          toggleFullscreen();
          break;
        case "Escape":
          if (drawer) setDrawer(false);
          else onClose();
          break;
        default:
          handled = false;
      }
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [togglePlay, seekBy, toggleFullscreen, st?.engine.volume, st?.engine.muted, drawer, onClose, live]);

  const seekTo = (clientX: number, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (clientX - r.left) / r.width));
    return x * dur;
  };

  const cycleSub = () => {
    const tracks = engine?.sub_tracks ?? [];
    if (!tracks.length) return;
    const cur = engine?.sub_id ?? -1;
    const i = tracks.findIndex((t) => t.id === cur);
    const next = i + 1 >= tracks.length ? -1 : tracks[i + 1].id;
    api.cmd("sub", { id: next });
  };

  const cycleAudio = () => {
    const tracks = engine?.audio_tracks ?? [];
    if (tracks.length < 2) return;
    const cur = engine?.audio_id ?? -1;
    const i = tracks.findIndex((t) => t.id === cur);
    const next = tracks[(i + 1) % tracks.length].id;
    api.cmd("audio", { id: next });
  };

  const cycleSpeed = () => {
    const cur = engine?.speed ?? 1;
    const i = SPEEDS.findIndex((s) => Math.abs(s - cur) < 0.01);
    const next = SPEEDS[(i + 1) % SPEEDS.length];
    api.cmd("speed", { value: next });
  };

  const vertical = kind !== "audio" && (engine?.height ?? 0) > (engine?.width ?? 0);
  const meta = engine
    ? [
        engine.width > 0 ? `${engine.width}×${engine.height}` : null,
        engine.vcodec || null,
        engine.hwdec ? `硬解 ${engine.hwdec}` : "软解",
        engine.container ? engine.container.toUpperCase() : null,
      ]
        .filter(Boolean)
        .join("  ·  ")
    : "准备中";

  const subLabel =
    engine && engine.sub_id != null
      ? engine.sub_tracks.find((t) => t.id === engine.sub_id)?.lang || "字幕"
      : "字幕";

  return (
    <div className="player">
      <div className="player-top">
        <button className="icon-btn" onClick={onClose} title="返回媒体库（Esc）">
          <Icon name="close" />
        </button>
        <div style={{ minWidth: 0, flex: 1 }}>
          <h3>
            {live && <span className="live-pill">直播</span>}
            {engine?.title || title}
          </h3>
          <div className="meta">{live ? "直播流 · 无进度" : meta}</div>
        </div>
        <div className="kbd-hint">
          <span>
            <b>空格</b> 播放
          </span>
          <span>
            <b>← →</b> 快退快进
          </span>
          <span>
            <b>F</b> 全屏
          </span>
          <span>
            <b>Esc</b> 退出
          </span>
        </div>
      </div>

      <div className="player-body">
        <div className="player-stage" onDoubleClick={toggleFullscreen}>
          {kind !== "audio" ? (
            <div
              ref={surfRef}
              className={`surface ${vertical ? "vertical" : "fill"}`}
              onClick={togglePlay}
            />
          ) : (
            <img className="music-cover" src={itemId ? art("item", itemId) : ""} alt={title} />
          )}
          {!engine?.available && (
            <div className="player-empty">
              播放内核未就绪。
              <br />
              {engine?.error || "请确认安装目录下的 libmpv-2.dll 存在。"}
            </div>
          )}
          {engine?.available && !engine.loaded && !engine.error && (
            <div className="player-loading">正在打开…</div>
          )}
          {engine?.error && <div className="player-empty">{engine.error}</div>}
        </div>

        <aside className={`eps-drawer ${drawer && !remote ? "open" : ""}`}>
          <header>
            <span>选集 · {queue.length}</span>
            <button
              className="cbtn"
              style={{ marginLeft: "auto" }}
              onClick={() => setDrawer(false)}
            >
              <Icon name="close" size={16} />
            </button>
          </header>
          <div className="list">
            {queue.map((it, i) => (
              <button
                key={it.id}
                className={`row-item ${it.id === itemId ? "on" : ""}`}
                onClick={() => {
                  api.open({ item_id: it.id });
                  setDrawer(false);
                }}
              >
                <span className="idx">{String(i + 1).padStart(2, "0")}</span>
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {epLabel(it)}
                </span>
                {it.finished && <Icon name="check" size={14} />}
              </button>
            ))}
          </div>
        </aside>
      </div>

      <div className="player-bottom">
        <div className="progress-row" style={live ? { display: "none" } : undefined}>
          <span className="time">{fmtTime(pos)}</span>
          <div
            className="track"
            onPointerDown={(e) => {
              const el = e.currentTarget;
              el.setPointerCapture(e.pointerId);
              setDrag(seekTo(e.clientX, el));
            }}
            onPointerMove={(e) => {
              if (drag == null) return;
              setDrag(seekTo(e.clientX, e.currentTarget));
            }}
            onPointerUp={(e) => {
              if (drag == null) return;
              api.cmd("seek", { value: seekTo(e.clientX, e.currentTarget) });
              setDrag(null);
            }}
          >
            <div className="track-rail">
              <div className="track-fill" style={{ width: `${pct}%` }} />
            </div>
            <div className="track-knob" style={{ left: `${pct}%` }} />
          </div>
          <span className="time right">{fmtTime(dur)}</span>
        </div>

        <div className="controls">
          {!remote && (
            <button className="cbtn" onClick={() => api.cmd("prev")} title="上一集">
              <Icon name="prev" />
            </button>
          )}
          <button className="cbtn" onClick={() => seekBy(-10)} title="后退 10 秒">
            <Icon name="back10" />
          </button>
          <button className="cbtn big" onClick={togglePlay} title="播放 / 暂停">
            <Icon name={engine?.playing ? "pause" : "play"} size={20} />
          </button>
          <button className="cbtn" onClick={() => seekBy(10)} title="前进 10 秒">
            <Icon name="fwd10" />
          </button>
          {!remote && (
            <button className="cbtn" onClick={() => api.cmd("next")} title="下一集">
              <Icon name="next" />
            </button>
          )}

          <div
            className="cbtn"
            style={{ width: 34, marginLeft: 6 }}
            onClick={() => api.cmd("mute", { value: engine?.muted ? 0 : 1 })}
            title="静音"
          >
            <Icon name={engine?.muted ? "mute" : "volume"} />
          </div>
          <div
            className="vol"
            onPointerDown={(e) => {
              const el = e.currentTarget;
              el.setPointerCapture(e.pointerId);
              const r = el.getBoundingClientRect();
              api.cmd("volume", {
                value: Math.round(Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * 130),
              });
            }}
          >
            <div className="vol-rail">
              <i style={{ width: `${Math.min(100, ((engine?.volume ?? 100) / 130) * 100)}%` }} />
            </div>
          </div>

          <div className="cfoot">
            <button className="speed-btn" onClick={cycleSpeed} title="播放速度">
              {engine?.speed && engine.speed !== 1 ? `${engine.speed}×` : "1.0×"}
            </button>
            <button className="cbtn" onClick={cycleAudio} title="切换音轨">
              <Icon name="music" size={16} />
            </button>
            <button
              className="speed-btn"
              onClick={cycleSub}
              title="切换字幕"
              style={{ color: engine?.sub_id != null ? "#fff" : undefined }}
            >
              {subLabel}
            </button>
            {!remote && (
              <button
                className="cbtn"
                onClick={() => setDrawer((v) => !v)}
                title="选集"
                style={{ color: drawer ? "#fff" : undefined }}
              >
                <Icon name="list" />
              </button>
            )}
            <button className="cbtn" onClick={toggleFullscreen} title="全屏">
              <Icon name="fullscreen" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
