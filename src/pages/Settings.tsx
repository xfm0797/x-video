import { useEffect, useState } from "react";
import { api, type PlayerPrefs, type Status } from "../lib/api";

type Props = {
  status: Status | null;
  toast: (msg: string) => void;
};

const MODES = [
  { key: "next", label: "播完下一集" },
  { key: "loop", label: "列表循环" },
  { key: "single", label: "单集循环" },
  { key: "stop", label: "播完停止" },
];

export function Settings({ status, toast }: Props) {
  const [prefs, setPrefs] = useState<PlayerPrefs | null>(null);

  useEffect(() => {
    api
      .settings()
      .then((r) => setPrefs(r.player))
      .catch(() => {});
  }, []);

  const save = async (patch: Partial<PlayerPrefs>) => {
    if (!prefs) return;
    const next = { ...prefs, ...patch };
    setPrefs(next);
    try {
      await api.saveSettings(next);
    } catch (e) {
      toast(String(e));
    }
  };

  return (
    <>
      <div className="panel">
        <h3>播放内核</h3>
        <p className="hint">
          内核是随包分发的 libmpv，支持 MP4 / MKV / AVI / RMVB / FLV / TS 以及
          H.264 / HEVC / AV1 / VP9 / RMVB 等编码，硬解优先。
        </p>
        <div className="kv">
          <span>状态</span>
          <b className={status?.engine.available ? "ok" : "bad"}>
            {status ? (status.engine.available ? "已就绪" : "不可用") : "检测中…"}
          </b>
        </div>
        <div className="kv">
          <span>内核路径</span>
          <b>{status?.engine.lib || "—"}</b>
        </div>
        {status?.engine.error && (
          <div className="kv">
            <span>错误</span>
            <b className="bad">{status.engine.error}</b>
          </div>
        )}
        <div className="kv">
          <span>版本</span>
          <b>X-Video {status?.version || "—"}</b>
        </div>
      </div>

      <div className="panel">
        <h3>播放行为</h3>
        <p className="hint">决定一部剧看完一集之后的行为。</p>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          {MODES.map((m) => (
            <button
              key={m.key}
              className={`chip ${prefs?.mode === m.key ? "on" : ""}`}
              onClick={() => save({ mode: m.key })}
            >
              {m.label}
            </button>
          ))}
        </div>
        <div style={{ marginTop: 18, display: "flex", alignItems: "center", gap: 14 }}>
          <span style={{ color: "var(--dim-2)", fontSize: 12.5 }}>默认音量</span>
          <input
            type="range"
            min={0}
            max={150}
            value={prefs?.volume ?? 100}
            onChange={(e) => setPrefs({ ...(prefs as PlayerPrefs), volume: Number(e.target.value) })}
            onPointerUp={() => save({ volume: prefs?.volume ?? 100 })}
            style={{ width: 220, accentColor: "#6f8cff" }}
          />
          <span style={{ fontSize: 12.5, color: "var(--dim)" }}>{Math.round(prefs?.volume ?? 100)}%</span>
        </div>
      </div>

      <div className="panel">
        <h3>数据</h3>
        <div className="kv">
          <span>存放位置</span>
          <b>{status?.data_dir || "—"}</b>
        </div>
        <div className="kv">
          <span>说明</span>
          <b style={{ color: "var(--dim)" }}>
            媒体索引 library.db、封面缓存 thumbs 都在这里，删除后不影响原始文件。
          </b>
        </div>
      </div>
    </>
  );
}
