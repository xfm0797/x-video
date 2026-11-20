import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type OnlinePlayback, type VodDetail, type VodHit, type VodSite } from "../lib/api";
import { fmtTime, progressPct } from "../lib/format";
import { Icon } from "../components/Icon";

type Props = {
  onPlay: (playback: OnlinePlayback) => Promise<boolean>;
  onGoSources: () => void;
  toast: (msg: string) => void;
};

const HISTORY_KEY = "xv.vod.search.history";
const HISTORY_MAX = 8;

function loadHistory(): string[] {
  try {
    const arr = JSON.parse(localStorage.getItem(HISTORY_KEY) || "[]");
    return Array.isArray(arr)
      ? arr.filter((x): x is string => typeof x === "string").slice(0, HISTORY_MAX)
      : [];
  } catch {
    return [];
  }
}

function saveHistory(list: string[]) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list));
  } catch {
    // 隐私模式等场景写入失败时静默忽略
  }
}

export function Online({ onPlay, onGoSources, toast }: Props) {
  // 支持 `?q=片名` 直达（方便截图与外部唤起）
  const bootQuery = new URLSearchParams(window.location.search).get("q") ?? "";
  const [sites, setSites] = useState<VodSite[]>([]);
  const [site, setSite] = useState("");
  const [kw, setKw] = useState(bootQuery);
  const [hits, setHits] = useState<VodHit[]>([]);
  const [failed, setFailed] = useState<{ site: string; error: string }[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [history, setHistory] = useState<string[]>(() => loadHistory());
  const [detail, setDetail] = useState<{ d: VodDetail; site: string } | null>(null);
  const [srcIdx, setSrcIdx] = useState(0);
  // 当前线路各集的已看进度（url -> { position, duration }），用于在剧集格上画进度条
  const [progress, setProgress] = useState<Record<string, { position: number; duration: number }>>(
    {}
  );

  useEffect(() => {
    api
      .vodSites()
      .then((r) => setSites(r.items))
      .catch(() => setSites([]));
  }, []);

  const usable = useMemo(() => sites.filter((s) => s.supported), [sites]);
  const usableCount = usable.length;

  const doSearch = useCallback(async (override?: string) => {
    const q = (override ?? kw).trim();
    if (!q) return;
    setLoading(true);
    setSearched(true);
    try {
      const r = await api.vodSearch(q, site || undefined);
      setHits(r.hits);
      setFailed(r.failed);
      setHistory((prev) => {
        const next = [q, ...prev.filter((x) => x !== q)].slice(0, HISTORY_MAX);
        saveHistory(next);
        return next;
      });
      if (!r.hits.length && r.failed.length) {
        toast(r.failed[0].error);
      }
    } catch (e) {
      toast(`搜索失败：${e instanceof Error ? e.message : String(e)}`);
      setHits([]);
    } finally {
      setLoading(false);
    }
  }, [kw, site, toast]);

  useEffect(() => {
    if (bootQuery.trim()) doSearch(bootQuery);
    // 仅在首屏执行一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openDetail = async (hit: VodHit) => {
    try {
      const r = await api.vodDetail(hit.site, hit.id);
      setDetail({ d: r.detail, site: hit.site });
      setSrcIdx(0);
    } catch (e) {
      toast(`详情加载失败：${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const playEpisode = async (url: string) => {
    if (!detail) return;
    const src = detail.d.sources[srcIdx];
    const episodes = (src?.episodes ?? []).map((e) => ({ name: e.name, url: e.url }));
    const index = Math.max(
      0,
      episodes.findIndex((e) => e.url === url)
    );
    const ok = await onPlay({
      title: detail.d.name,
      site: detail.site,
      flag: src?.from ?? "",
      episodes,
      index,
    });
    if (ok) setDetail(null);
  };

  // 详情或线路切换时，批量拉取当前线路各集的已看进度
  const loadProgress = useCallback(async () => {
    if (!detail) {
      setProgress({});
      return;
    }
    const src = detail.d.sources[srcIdx];
    const eps = src?.episodes ?? [];
    if (!eps.length) {
      setProgress({});
      return;
    }
    const items = eps.map((e) => ({ site: detail.site, flag: src?.from ?? "", url: e.url }));
    try {
      const r = await api.onlineProgress(items);
      const map: Record<string, { position: number; duration: number }> = {};
      for (const it of r.items) map[it.url] = { position: it.position, duration: it.duration };
      setProgress(map);
    } catch {
      setProgress({});
    }
  }, [detail, srcIdx]);

  useEffect(() => {
    loadProgress();
  }, [loadProgress]);

  if (sites.length === 0) {
    return (
      <div className="empty-state">
        <div className="empty-art">
          <Icon name="cloud" size={30} />
        </div>
        <h3>没有可用的点播站点</h3>
        <p>
          TVBox 配置里的 CMS 站点（type=1）可以直接在线搜索播放。添加一份配置后即可使用。
        </p>
        <button className="btn-primary" onClick={onGoSources}>
          去添加订阅
        </button>
      </div>
    );
  }

  const activeSources = detail?.d.sources ?? [];
  const episodes = activeSources[srcIdx]?.episodes ?? [];

  return (
    <div className="online">
      <div className="online-bar">
        <div className="search">
          <Icon name="search" size={15} />
          <input
            value={kw}
            onChange={(e) => setKw(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && doSearch()}
            placeholder="搜索片名，回车跨站搜索"
            spellCheck={false}
          />
        </div>
        <button className="btn-primary" onClick={() => doSearch()} disabled={loading || !kw.trim()}>
          {loading ? "搜索中…" : "搜索"}
        </button>
      </div>

      <div className="chips site-chips">
        <button className={`chip ${site === "" ? "on" : ""}`} onClick={() => setSite("")}>
          全部站点
        </button>
        {usable.map((s) => (
          <button
            key={`${s.sub_id}|${s.key}`}
            className={`chip ${site === `${s.sub_id}|${s.key}` ? "on" : ""}`}
            onClick={() => setSite(`${s.sub_id}|${s.key}`)}
          >
            {s.name}
            <i className="chip-sub">{s.sub_name}</i>
          </button>
        ))}
        {sites.length > usableCount && (
          <span className="hint dim">
            {sites.length - usableCount} 个站点需要 Java spider 内核，暂不支持
          </span>
        )}
      </div>

      {loading && (
        <div className="grid movies">
          {Array.from({ length: 8 }).map((_, i) => (
            <div className="card movies" key={i}>
              <div className="card-pic skel" />
              <div className="skel-line" />
            </div>
          ))}
        </div>
      )}

      {!loading && hits.length > 0 && (
        <>
          <div className="section-head">
            <h2>搜索结果</h2>
            <span className="count">{hits.length}</span>
            {failed.length > 0 && <span className="hint dim">{failed.length} 个站点失败</span>}
          </div>
          <div className="grid movies">
            {hits.map((h) => (
              <button
                key={`${h.site}-${h.id}`}
                className="card movies"
                onClick={() => openDetail(h)}
              >
                <div className="card-pic">
                  {h.pic ? (
                    <img src={h.pic} alt="" loading="lazy" />
                  ) : (
                    <div className="poster-fallback">{h.name.slice(0, 2)}</div>
                  )}
                  <span className="card-play">
                    <span>
                      <Icon name="play" size={18} />
                    </span>
                  </span>
                </div>
                <div className="card-name">{h.name}</div>
                <div className="card-sub">
                  {[h.remarks, h.year].filter(Boolean).join(" · ") || h.site_name}
                </div>
              </button>
            ))}
          </div>
        </>
      )}

      {!loading && !searched && history.length > 0 && (
        <div className="chips" style={{ marginTop: 6 }}>
          <span className="hint dim" style={{ marginRight: 2 }}>
            最近搜索
          </span>
          {history.map((h) => (
            <button
              key={h}
              className="chip"
              onClick={() => {
                setKw(h);
                doSearch(h);
              }}
            >
              {h}
            </button>
          ))}
          <button
            className="chip"
            onClick={() => {
              setHistory([]);
              saveHistory([]);
            }}
          >
            清除
          </button>
        </div>
      )}

      {!loading && !searched && (
        <div className="empty-state">
          <div className="empty-art">
            <Icon name="search" size={28} />
          </div>
          <h3>输入片名开始搜索</h3>
          <p>会同时在 {usableCount} 个可用站点里找片，点开海报即可选集播放。</p>
        </div>
      )}

      {!loading && searched && hits.length === 0 && (
        <div className="empty">没有搜到结果，换个片名或站点试试</div>
      )}

      {detail && (
        <div className="sheet-mask" onClick={() => setDetail(null)}>
          <div className="sheet" onClick={(e) => e.stopPropagation()}>
            <button className="sheet-close" onClick={() => setDetail(null)}>
              <Icon name="close" size={16} />
            </button>
            <div className="sheet-head">
              <div className="sheet-poster">
                {detail.d.pic ? (
                  <img src={detail.d.pic} alt="" />
                ) : (
                  <div className="poster-fallback">{detail.d.name.slice(0, 2)}</div>
                )}
              </div>
              <div className="sheet-info">
                <h2>{detail.d.name}</h2>
                <div className="meta-line">
                  {[detail.d.year, detail.d.area, detail.d.kind].filter(Boolean).join(" · ")}
                </div>
                {detail.d.actor && <p className="dim">主演：{detail.d.actor}</p>}
                {detail.d.director && <p className="dim">导演：{detail.d.director}</p>}
                {detail.d.content && <p className="synopsis">{detail.d.content}</p>}
              </div>
            </div>

            {activeSources.length > 1 && (
              <div className="source-tabs">
                {activeSources.map((s, i) => (
                  <button
                    key={s.from}
                    className={`seg-item ${i === srcIdx ? "on" : ""}`}
                    onClick={() => setSrcIdx(i)}
                  >
                    {s.from}
                  </button>
                ))}
              </div>
            )}

            <div className="ep-grid">
              {episodes.map((ep) => {
                const p = progress[ep.url];
                const pct = p ? progressPct(p.position, p.duration) : 0;
                return (
                  <button
                    key={`${ep.name}-${ep.url}`}
                    className={`ep${p ? " watched" : ""}`}
                    onClick={() => playEpisode(ep.url)}
                    title={
                      p
                        ? `已看 ${fmtTime(p.position)} / ${fmtTime(p.duration)}，重开将自动续播`
                        : ep.name
                    }
                  >
                    {ep.name}
                    {p && <i className="seen" />}
                    {pct > 0 && (
                      <span className="prog">
                        <i style={{ width: `${pct}%` }} />
                      </span>
                    )}
                  </button>
                );
              })}
              {episodes.length === 0 && <div className="empty">这一线路没有剧集</div>}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
