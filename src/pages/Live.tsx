import { useEffect, useMemo, useState } from "react";
import { api, type LiveGroup, type Source } from "../lib/api";
import Player from "../components/Player";

interface Props {
  sources: Source[];
}

export default function Live({ sources }: Props) {
  const liveSources = useMemo(
    () => sources.filter((s) => s.channel_count > 0),
    [sources]
  );
  const [sourceId, setSourceId] = useState<string>("");
  const [groups, setGroups] = useState<LiveGroup[]>([]);
  const [activeGroup, setActiveGroup] = useState<string>("");
  const [keyword, setKeyword] = useState("");
  const [playing, setPlaying] = useState<{
    url: string;
    kind: string;
    name: string;
  } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!sourceId && liveSources.length > 0) setSourceId(liveSources[0].id);
  }, [liveSources, sourceId]);

  useEffect(() => {
    if (!sourceId) return;
    setLoading(true);
    setError(null);
    setGroups([]);
    api
      .get<LiveGroup[]>(`/api/live/${sourceId}`)
      .then((gs) => {
        setGroups(gs);
        if (gs.length > 0) setActiveGroup(gs[0].name);
      })
      .catch((e) => setError((e as Error).message))
      .finally(() => setLoading(false));
  }, [sourceId]);

  const play = async (name: string, url: string) => {
    try {
      const p = await api.get<{ url: string; kind: string }>(
        `/api/play/live?u=${encodeURIComponent(url)}`
      );
      setPlaying({ url: p.url, kind: p.kind, name });
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const channels = useMemo(() => {
    let list: { group: string; name: string; url: string }[] = [];
    for (const g of groups) {
      if (keyword.trim()) {
        if (g.name.includes(keyword.trim())) {
          for (const c of g.channels)
            list.push({ group: g.name, name: c.name, url: c.url });
        }
        for (const c of g.channels) {
          if (c.name.toLowerCase().includes(keyword.trim().toLowerCase())) {
            list.push({ group: g.name, name: c.name, url: c.url });
          }
        }
      } else if (g.name === activeGroup) {
        for (const c of g.channels)
          list.push({ group: g.name, name: c.name, url: c.url });
      }
    }
    // 去重
    const seen = new Set<string>();
    list = list.filter((x) => {
      const key = x.group + "|" + x.name + "|" + x.url;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    return list.slice(0, 500);
  }, [groups, activeGroup, keyword]);

  return (
    <div className="page live-page">
      <div className="live-toolbar">
        <h2>直播</h2>
        <select
          value={sourceId}
          onChange={(e) => setSourceId(e.target.value)}
          disabled={liveSources.length === 0}
        >
          {liveSources.length === 0 && <option value="">无可用直播源</option>}
          {liveSources.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}（{s.channel_count}）
            </option>
          ))}
        </select>
        <input
          className="search"
          placeholder="搜索频道…"
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
        />
      </div>

      {loading && <div className="empty">加载频道中…</div>}
      {error && <div className="msg err">{error}</div>}

      {!loading && groups.length > 0 && (
        <div className="live-layout">
          <div className="group-col">
            {groups.map((g) => (
              <div
                key={g.name}
                className={`group-item${g.name === activeGroup && !keyword ? " active" : ""}`}
                onClick={() => {
                  setActiveGroup(g.name);
                  setKeyword("");
                }}
              >
                {g.name}
                <span className="count">{g.channels.length}</span>
              </div>
            ))}
          </div>
          <div className="channel-col">
            {channels.map((c) => (
              <div
                key={c.group + c.url}
                className="channel-item"
                onClick={() => play(c.name, c.url)}
              >
                <span className="channel-name">{c.name}</span>
                <span className="channel-group">{c.group}</span>
              </div>
            ))}
            {channels.length === 0 && (
              <div className="empty">该分组暂无频道</div>
            )}
          </div>
        </div>
      )}

      {playing && (
        <Player
          url={playing.url}
          kind={playing.kind}
          title={playing.name}
          onClose={() => setPlaying(null)}
        />
      )}
    </div>
  );
}
