import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";

interface Props {
  url: string;
  kind: string;
  title: string;
  onClose: () => void;
}

export default function Player({ url, kind, title, onClose }: Props) {
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
      </div>
      <div className="player-stage">
        {loading && <div className="player-loading">加载中…</div>}
        {error && <div className="player-error">{error}</div>}
        <video
          ref={videoRef}
          controls
          autoPlay
          onPlaying={() => setLoading(false)}
          onError={() => {
            setLoading(false);
            setError(
              "播放失败：格式可能不受支持（FLV/RMVB 等需 libmpv 内核，规划中）"
            );
          }}
        />
      </div>
    </div>
  );
}
