import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type LiveChannel, type LiveGroup, type Sub } from "../lib/api";
import { Icon } from "../components/Icon";

type Props = {
  onPlay: (ch: { url: string; name: string; urls?: string[]; headers?: string }) => void;
  onGoSources: () => void;
  toast: (msg: string) => void;
};

type GroupView = { name: string; channels: LiveChannel[] };

export function Live({ onPlay, onGoSources, toast }: Props) {
  const [groups, setGroups] = useState<LiveGroup[]>([]);
  const [subs, setSubs] = useState<Sub[]>([]);
  const [sub, setSub] = useState("");
  const [group, setGroup] = useState("");
  const [kw, setKw] = useState("");
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const [l, s] = await Promise.all([api.live(), api.sources()]);
      setGroups(l.groups);
      setSubs(s.items);
    } catch (e) {
      toast(`频道加载失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  const subOptions = useMemo(() => {
    const seen = new Set<string>();
    return subs.filter((s) => {
      if (seen.has(s.url)) return false;
      if (!groups.some((g) => g.sub_id === s.id)) return false;
      seen.add(s.url);
      return true;
    });
  }, [subs, groups]);

  // 多订阅下同名分组（如「央视」）合并成一组，重复地址只留一条。
  const visibleGroups = useMemo<GroupView[]>(() => {
    const map = new Map<string, LiveChannel[]>();
    for (const g of groups) {
      if (sub && g.sub_id !== sub) continue;
      if (g.channels.length === 0) continue;
      const list = map.get(g.group) ?? [];
      for (const c of g.channels) {
        if (!list.some((x) => x.url === c.url)) list.push(c);
      }
      map.set(g.group, list);
    }
    return [...map.entries()]
      .map(([name, channels]) => ({ name, channels }))
      .sort((a, b) => b.channels.length - a.channels.length);
  }, [groups, sub]);

  const active = visibleGroups.find((g) => g.name === group) ?? visibleGroups[0];
  const channels: LiveChannel[] = useMemo(() => {
    const list = active?.channels ?? [];
    const k = kw.trim().toLowerCase();
    return k ? list.filter((c) => c.name.toLowerCase().includes(k)) : list;
  }, [active, kw]);

  const total = visibleGroups.reduce((n, g) => n + g.channels.length, 0);

  if (!loading && groups.length === 0) {
    return (
      <div className="empty-state">
        <div className="empty-art">
          <Icon name="live" size={30} />
        </div>
        <h3>还没有直播源</h3>
        <p>
          添加一条 IPTV 订阅（m3u / txt）或 TVBox 配置，就能在这里看央视、卫视和地方台。
        </p>
        <button className="btn-primary" onClick={onGoSources}>
          去添加订阅
        </button>
      </div>
    );
  }

  return (
    <div className="live">
      <div className="live-bar">
        {subOptions.length > 1 && (
          <div className="seg">
            <button className={`seg-item ${sub === "" ? "on" : ""}`} onClick={() => setSub("")}>
              全部
            </button>
            {subOptions.map((s) => (
              <button
                key={s.id}
                className={`seg-item ${sub === s.id ? "on" : ""}`}
                onClick={() => setSub(s.id)}
              >
                {s.name}
              </button>
            ))}
          </div>
        )}
        <div className="search slim">
          <Icon name="search" size={14} />
          <input
            value={kw}
            onChange={(e) => setKw(e.target.value)}
            placeholder="搜频道"
            spellCheck={false}
          />
        </div>
        <span className="live-count">{total} 个频道</span>
        <button className="icon-btn" onClick={load} title="重新读取">
          <Icon name="refresh" />
        </button>
      </div>

      <div className="live-body">
        <aside className="live-groups">
          {visibleGroups.map((g) => (
            <button
              key={g.name}
              className={`group-item ${g.name === active?.name ? "on" : ""}`}
              onClick={() => setGroup(g.name)}
            >
              <span>{g.name}</span>
              <i>{g.channels.length}</i>
            </button>
          ))}
        </aside>

        <div className="live-panel">
          <div className="live-panel-head">
            <h2>{active?.name || "频道"}</h2>
            <span className="hint">{channels.length} 个</span>
          </div>
          <div className="chan-grid">
            {channels.map((c) => {
              // 同名频道往往是同一频道的多个源（IPTV 常见），点击时全部交给
              // 后端按顺序尝试：第一个挂了自动换下一个，用户无感。
              const urls = (active?.channels ?? [])
                .filter((x) => x.name === c.name)
                .map((x) => x.url);
              return (
                <button
                  key={`${c.name}-${c.url}`}
                  className="chan"
                  onClick={() =>
                    onPlay({ url: c.url, name: c.name, urls, headers: c.headers ?? "" })
                  }
                  title={c.url}
                >
                  <span className="chan-logo">
                    {c.logo ? (
                      <img
                        src={c.logo}
                        alt=""
                        loading="lazy"
                        onError={(e) => {
                          e.currentTarget.style.display = "none";
                        }}
                      />
                    ) : (
                      <b>{c.name.slice(0, 2)}</b>
                    )}
                  </span>
                  <span className="chan-name">{c.name}</span>
                </button>
              );
            })}
            {channels.length === 0 && <div className="empty">没有匹配的频道</div>}
          </div>
        </div>
      </div>
    </div>
  );
}
