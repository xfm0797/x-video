import { useCallback, useEffect, useState } from "react";
import { api, type Sub } from "../lib/api";
import { fmtRelative } from "../lib/format";
import { Icon } from "../components/Icon";

type Props = {
  toast: (msg: string) => void;
  onChanged: () => void;
  onGoLive: () => void;
};

const PRESETS = [
  {
    name: "IPTV · 中国频道",
    kind: "iptv",
    url: "https://iptv-org.github.io/iptv/countries/cn.m3u",
    note: "央视 / 卫视 / 地方台，公共维护",
  },
  {
    name: "TVBox · 0825 配置",
    kind: "tvbox",
    url: "https://cdn.jsdelivr.net/gh/gaotianliuyun/gao@master/0825.json",
    note: "含 CMS 点播站与直播分组",
  },
];

export function Sources({ toast, onChanged, onGoLive }: Props) {
  const [subs, setSubs] = useState<Sub[]>([]);
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [kind, setKind] = useState("auto");
  const [busy, setBusy] = useState("");
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const r = await api.sources();
      setSubs(r.items);
    } catch (e) {
      toast(`读取订阅失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  const add = async (address: string, title = "", k = kind) => {
    const u = address.trim();
    if (!u) return;
    setBusy(u);
    try {
      const r = await api.addSource({ url: u, name: title, kind: k });
      setSubs(r.items);
      setUrl("");
      setName("");
      const sub = r.sub;
      if (sub.error) toast(`已添加，但拉取失败：${sub.error}`);
      else toast(`已添加：${sub.summary || "解析完成"}`);
      onChanged();
    } catch (e) {
      toast(`添加失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy("");
    }
  };

  const refresh = async (id: string) => {
    setBusy(id);
    try {
      const r = await api.refreshSources(id);
      setSubs(r.items);
      const s = r.items.find((x) => x.id === id);
      toast(s?.error ? `刷新失败：${s.error}` : `已刷新：${s?.summary ?? ""}`);
      onChanged();
    } catch (e) {
      toast(`刷新失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy("");
    }
  };

  const remove = async (s: Sub) => {
    if (!window.confirm(`删除订阅「${s.name}」？`)) return;
    try {
      const r = await api.removeSource(s.id);
      setSubs(r.items);
      onChanged();
    } catch (e) {
      toast(`删除失败：${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const toggle = async (s: Sub) => {
    try {
      const r = await api.toggleSource(s.id, !s.enabled);
      setSubs(r.items);
      onChanged();
    } catch (e) {
      toast(`操作失败：${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <div className="sources">
      <section className="panel">
        <header className="panel-head">
          <h2>添加订阅</h2>
          <span className="hint">支持 IPTV 直播清单（m3u / txt）与 TVBox 配置（json）</span>
        </header>

        <div className="add-row">
          <input
            className="input"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add(url)}
            placeholder="https://example.com/live.m3u  或  https://example.com/tvbox.json"
            spellCheck={false}
          />
          <input
            className="input narrow"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="备注名（可选）"
          />
          <select className="select" value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="auto">自动识别</option>
            <option value="iptv">IPTV 直播</option>
            <option value="tvbox">TVBox 配置</option>
          </select>
          <button className="btn-primary" disabled={!url.trim() || !!busy} onClick={() => add(url)}>
            {busy === url.trim() ? "解析中…" : "添加"}
          </button>
        </div>

        <div className="preset-row">
          <span className="hint dim">常用：</span>
          {PRESETS.map((p) => (
            <button
              key={p.url}
              className="preset"
              disabled={!!busy}
              onClick={() => add(p.url, p.name, p.kind)}
              title={p.url}
            >
              <b>{p.name}</b>
              <i>{p.note}</i>
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <header className="panel-head">
          <h2>我的订阅</h2>
          <span className="hint">
            {subs.length} 条 · 直播 {subs.reduce((n, s) => n + s.channels, 0)} 频道 · 站点{" "}
            {subs.reduce((n, s) => n + s.sites, 0)} 个
          </span>
          <button
            className="btn-ghost"
            style={{ marginLeft: "auto" }}
            disabled={!!busy || subs.length === 0}
            onClick={async () => {
              setBusy("all");
              try {
                const r = await api.refreshSources("");
                setSubs(r.items);
                toast("已刷新全部订阅");
                onChanged();
              } finally {
                setBusy("");
              }
            }}
          >
            <Icon name="refresh" size={14} /> 全部刷新
          </button>
        </header>

        {loading && <div className="empty">读取中…</div>}
        {!loading && subs.length === 0 && (
          <div className="empty">还没有订阅，先从上面的常用源开始</div>
        )}

        <div className="sub-list">
          {subs.map((s) => (
            <div className={`sub-card ${s.enabled ? "" : "off"}`} key={s.id}>
              <div className="sub-badge">{s.kind === "tvbox" ? "TVBox" : "IPTV"}</div>
              <div className="sub-main">
                <div className="sub-title">
                  <b>{s.name}</b>
                  {s.error ? (
                    <span className="tag err">拉取失败</span>
                  ) : (
                    <span className="tag ok">{s.summary || "待刷新"}</span>
                  )}
                </div>
                <div className="sub-url">{s.url}</div>
                <div className="sub-foot">
                  <span>
                    {s.kind === "tvbox"
                      ? `${s.sites} 站点 · ${s.channels} 直播频道`
                      : `${s.groups} 组 · ${s.channels} 频道`}
                  </span>
                  <span className="dim">
                    {s.updated_at ? `更新于 ${fmtRelative(s.updated_at)}` : "尚未成功拉取"}
                  </span>
                </div>
                {s.error && <div className="sub-err">{s.error}</div>}
              </div>
              <div className="sub-actions">
                {s.channels > 0 && (
                  <button className="mini" onClick={onGoLive} title="去看直播">
                    <Icon name="live" size={14} /> 直播
                  </button>
                )}
                <button className="mini" disabled={busy === s.id} onClick={() => refresh(s.id)}>
                  <Icon name="refresh" size={14} /> 刷新
                </button>
                <button className="mini" onClick={() => toggle(s)}>
                  {s.enabled ? "停用" : "启用"}
                </button>
                <button className="mini danger" onClick={() => remove(s)}>
                  <Icon name="trash" size={14} />
                </button>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel note">
        <h3>能力说明</h3>
        <ul>
          <li>
            <b>IPTV</b>：m3u / m3u8 / txt 直播清单，自动识别分组、台标与频道自带的 Referer。
          </li>
          <li>
            <b>TVBox · CMS 站点</b>：type=1 的 JSON 接口站，支持跨站搜索、选集播放；分享页会自动抽取真实
            m3u8 地址。
          </li>
          <li>
            <b>TVBox · spider 站点</b>：type=3 里写成 JS 规则的（drpy 脚本）由内置 JS 引擎执行，支持选择器
            解析与 <code>js:</code> 代码片段；<code>csp_*</code>（需要 pg.jar 的 Java 内核）暂无运行环境，
            列表里会标注不支持。
          </li>
          <li>订阅与解析结果保存在本地数据目录的 sources.json，重启后仍在。</li>
        </ul>
      </section>
    </div>
  );
}
