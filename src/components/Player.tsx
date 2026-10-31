import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";
import MpvPlayer from "./MpvPlayer";
import { needsMpv } from "../lib/mpv";

interface Props {
  url: string;
  kind: string;
  title: string;
  onClose: () => void;
}

type Engine = "web" | "mpv";

/**
 * 播放器外壳：默认走浏览器内核（hls.js / <video>），
 * 遇到 FLV 或浏览器解不了的容器时自动切到 mpv 内核。
 */
export default function Player({ url, kind, title, onClose }: Props) {
  const [engine, setEngine] = useState<Engine>(() =>
    needsMpv(kind) ? "mpv" : "web"
  );
  const [fallbackNote, setFallbackNote] = useState<string | null>(null);

  // 地址变化时重置内核选择
  useEffect(() => {
    setEngine(needsMpv(kind) ? "mpv" : "web");
    setFallbackNote(null);
  }, [url, kind]);

  const toMpv = (reason: string) => {
    setFallbackNote(reason);
    setEngine("mpv");
  };

  if (engine === "mpv") {
    return (
      <MpvPlayer
        url={url}
        title={title}
        onClose={onClose}
        onSwitchWeb={() => setEngine("web")}
      />
    );
  }

  return (
    <WebPlayer
      url={url}
      kind={kind}
      title={title}
      note={fallbackNote}
      onClose={onClose}
      onUseMpv={toMpv}
    />
  );
}

interface WebProps {
  url: string;
  kind: string;
  title: string;
  note: string | null;
  onClose: () => void;
  onUseMpv: (reason: string) => void;
}

function WebPlayer({ url, kind, title, note, onClose, onUseMpv }: WebProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    setError(null);
    setLoading(true);

    if (kind === "hls" && Hls.isSupported()) {
      const instance = new Hls({ lowLatencyMode: false, maxBufferLength: 30 });
      hlsRef.current = instance;
      instance.loadSource(url);
      instance.attachMedia(video);
      instance.on(Hls.Events.ERROR, (_evt, data) => {
        if (data.fatal) {
          setError(`HLS 播放失败: ${data.details ?? data.type}`);
          setLoading(false);
        }
      });
      instance.on(Hls.Events.MANIFEST_PARSED, () => {
        video.play().catch(() => {});
      });
    } else {
      video.src = url;
      video.play().catch(() => {});
    }

    return () => {
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
      video.removeAttribute("src");
      video.load();
    };
  }, [url, kind]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  };

  return (
    <div className="player-overlay" tabIndex={0} onKeyDown={onKey}>
      <div className="player-head">
        <button className="btn ghost" onClick={onClose}>
          ← 返回
        </button>
        <span className="player-title">{title}</span>
        <span className={`badge kind-${kind}`}>{kind}</span>
        <button
          className="btn ghost"
          onClick={() => onUseMpv("已手动切换到 mpv 内核")}
        >
          MPV 内核
        </button>
      </div>
      <div className="player-stage">
        {loading && <div className="player-loading">加载中…</div>}
        {error && <div className="player-error">{error}</div>}
        {note && !error && <div className="player-note">{note}</div>}
        <video
          ref={videoRef}
          controls
          autoPlay
          onPlaying={() => setLoading(false)}
          onError={() => {
            setLoading(false);
            onUseMpv("浏览器内核不支持该格式，已自动切换到 mpv 内核");
          }}
        />
      </div>
    </div>
  );
}
