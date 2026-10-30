import { useState } from "react";
import { api, KIND_LABEL, type Source, type SourceKind } from "../lib/api";

interface Props {
  sources: Source[];
  onChanged: () => void;
}

export default function Sources({ sources, onChanged }: Props) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<SourceKind>("m3u");
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [refreshing, setRefreshing] = useState<string | null>(null);

  const add = async () => {
    if (!name.trim() || !url.trim()) {
      setMsg({ ok: false, text: "请填写名称与地址" });
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      const s = await api.post<Source>("/api/sources", {
        name: name.trim(),
        type: kind,
        url: url.trim(),
      });
      setMsg({
        ok: true,
        text: `已添加「${s.name}」（频道 ${s.channel_count} / 站点 ${s.site_count}）`,
      });
      setName("");
      setUrl("");
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const refresh = async (id: string) => {
    setRefreshing(id);
    try {
      const s = await api.post<Source>(`/api/sources/${id}/refresh`);
      setMsg({
        ok: true,
        text: `「${s.name}」刷新成功（频道 ${s.channel_count} / 站点 ${s.site_count}）`,
      });
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setRefreshing(null);
    }
  };

  const remove = async (s: Source) => {
    if (!confirm(`确认删除源「${s.name}」？`)) return;
    try {
      await api.del(`/api/sources/${s.id}`);
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    }
  };

  return (
    <div className="page">
      <h2>源管理</h2>
      <p className="hint">
        支持 M3U/M3U8 直播列表、TXT 直播列表与 TVBox 配置（点播 + 直播）。
      </p>

      <div className="card add-form">
        <div className="row">
          <input
            placeholder="源名称，如：我的 IPTV"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <select value={kind} onChange={(e) => setKind(e.target.value as SourceKind)}>
            <option value="m3u">M3U 直播</option>
            <option value="txt">TXT 直播</option>
            <option value="tvbox">TVBox 配置</option>
          </select>
        </div>
        <div className="row">
          <input
            placeholder="订阅地址（http/https）"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
          <button className="btn primary" disabled={busy} onClick={add}>
            {busy ? "校验中…" : "添加源"}
          </button>
        </div>
        {msg && <div className={msg.ok ? "msg ok" : "msg err"}>{msg.text}</div>}
      </div>

      <div className="source-list">
        {sources.length === 0 && (
          <div className="empty">还没有源，先添加一个试试。</div>
        )}
        {sources.map((s) => (
          <div className="card source-item" key={s.id}>
            <div className="source-main">
              <div className="source-name">
                {s.name}
                <span className="badge">{KIND_LABEL[s.type]}</span>
              </div>
              <div className="source-url" title={s.url}>
                {s.url}
              </div>
              <div className="source-meta">
                {s.type === "tvbox"
                  ? `站点 ${s.site_count} · 直播频道 ${s.channel_count}`
                  : `频道 ${s.channel_count}`}
                {s.last_error && (
                  <span className="err-text"> · 上次错误：{s.last_error}</span>
                )}
              </div>
            </div>
            <div className="source-actions">
              <button
                className="btn"
                disabled={refreshing === s.id}
                onClick={() => refresh(s.id)}
              >
                {refreshing === s.id ? "刷新中…" : "刷新"}
              </button>
              <button className="btn danger" onClick={() => remove(s)}>
                删除
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
