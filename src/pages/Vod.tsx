import { useEffect, useMemo, useState } from "react";
import {
  api,
  type Source,
  type TvBoxSite,
  type VodCategory,
  type VodItem,
  type VodDetail,
  type AggResult,
} from "../lib/api";
import Player from "../components/Player";

interface Props {
  sources: Source[];
}

export default function Vod({ sources }: Props) {
  const tvboxSources = useMemo(
    () => sources.filter((s) => s.type === "tvbox" && s.site_count > 0),
    [sources]
  );
  const [sourceId, setSourceId] = useState("");
  const [sites, setSites] = useState<TvBoxSite[]>([]);
  const [siteKey, setSiteKey] = useState("");
  const [cats, setCats] = useState<VodCategory[]>([]);
  const [catId, setCatId] = useState("");
  const [items, setItems] = useState<VodItem[]>([]);
  const [pg, setPg] = useState(1);
  const [pageCount, setPageCount] = useState(1);
  const [detail, setDetail] = useState<VodDetail | null>(null);
  const [playing, setPlaying] = useState<{ url: string; title: string } | null>(
    null
  );
  const [keyword, setKeyword] = useState("");
  const [searchResult, setSearchResult] = useState<VodItem[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [parsing, setParsing] = useState(false);
  const [agg, setAgg] = useState<AggResult | null>(null);
  const [aggLoading, setAggLoading] = useState(false);
  const [detailCtx, setDetailCtx] = useState<{ sid: string; skey: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!sourceId && tvboxSources.length > 0) setSourceId(tvboxSources[0].id);
  }, [tvboxSources, sourceId]);

  // 站点列表
  useEffect(() => {
    if (!sourceId) return;
    setError(null);
    api
      .get<{ sites: TvBoxSite[] }>(`/api/tvbox/${sourceId}/sites`)
      .then((d) => {
        setSites(d.sites);
        const firstSupported = d.sites.find((s) => s.site_type === 1) ?? d.sites[0];
        setSiteKey(firstSupported?.key ?? "");
      })
      .catch((e) => setError((e as Error).message));
  }, [sourceId]);

  // 分类
  useEffect(() => {
    if (!sourceId || !siteKey) return;
    setCats([]);
    setItems([]);
    setSearchResult(null);
    api
      .get<VodCategory[]>(`/api/cms/${sourceId}/${siteKey}/home`)
      .then((cs) => {
        setCats(cs);
        if (cs.length > 0) setCatId(cs[0].type_id);
      })
      .catch((e) => setError((e as Error).message));
  }, [sourceId, siteKey]);

  // 列表
  useEffect(() => {
    if (!sourceId || !siteKey || !catId || searchResult) return;
    setLoading(true);
    setError(null);
    api
      .get<{ list: VodItem[]; pagecount: number }>(
        `/api/cms/${sourceId}/${siteKey}/list?tid=${encodeURIComponent(catId)}&pg=${pg}`
      )
      .then((d) => {
        setItems(d.list);
        setPageCount(Math.max(1, d.pagecount));
      })
      .catch((e) => setError((e as Error).message))
      .finally(() => setLoading(false));
  }, [sourceId, siteKey, catId, pg, searchResult]);

  const openDetail = async (vodId: string, sid = sourceId, skey = siteKey) => {
    setLoading(true);
    setError(null);
    try {
      const d = await api.get<VodDetail>(
        `/api/cms/${sid}/${skey}/detail?vod_id=${encodeURIComponent(vodId)}`
      );
      setDetail(d);
      setDetailCtx({ sid, skey });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  const playEpisode = async (epName: string, rawUrl: string) => {
    if (!detail) return;
    setParsing(true);
    setError(null);
    try {
      const sid = detailCtx?.sid ?? sourceId;
      const skey = detailCtx?.skey ?? siteKey;
      const p = await api.get<{ url: string; kind: string }>(
        `/api/cms/${sid}/${skey}/play?url=${encodeURIComponent(rawUrl)}`
      );
      setPlaying({ url: p.url, title: `${detail.vod_name} · ${epName}` });
    } catch (e) {
      setError(`解析失败：${(e as Error).message}`);
    } finally {
      setParsing(false);
    }
  };

  const doAggSearch = async () => {
    const kw = keyword.trim();
    if (!kw) return;
    setAggLoading(true);
    setError(null);
    setAgg(null);
    setSearchResult(null);
    try {
      const r = await api.get<AggResult>(`/api/search?wd=${encodeURIComponent(kw)}`);
      setAgg(r);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setAggLoading(false);
    }
  };

  const doSearch = async () => {
    if (!keyword.trim() || !sourceId || !siteKey) return;
    setLoading(true);
    setError(null);
    try {
      const list = await api.get<VodItem[]>(
        `/api/cms/${sourceId}/${siteKey}/search?wd=${encodeURIComponent(keyword.trim())}`
      );
      setSearchResult(list);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  };

  const shown = searchResult ?? items;

  return (
    <div className="page vod-page">
      <div className="live-toolbar">
        <h2>点播</h2>
        <select value={sourceId} onChange={(e) => { setSourceId(e.target.value); setPg(1); }}>
          {tvboxSources.length === 0 && <option value="">无可用 TVBox 源</option>}
          {tvboxSources.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}（{s.site_count} 站点）
            </option>
          ))}
        </select>
        <select value={siteKey} onChange={(e) => { setSiteKey(e.target.value); setSearchResult(null); setPg(1); }}>
          {sites.map((s) => (
            <option key={s.key} value={s.key}>
              {s.name}
              {s.site_type === 1
                ? ""
                : s.site_type === 3
                ? "（JS Spider）"
                : `（type=${s.site_type} 暂不支持）`}
            </option>
          ))}
        </select>
        <input
          className="search"
          placeholder="搜索影片…"
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && doSearch()}
        />
        <button className="btn" onClick={doSearch}>
          本站搜索
        </button>
        <button className="btn primary" onClick={doAggSearch} disabled={aggLoading}>
          {aggLoading ? "全网搜索中…" : "全网搜索"}
        </button>
      </div>

      {agg && (
        <div className="agg-panel">
          <div className="agg-head">
            <span className="badge">
              「{keyword}」命中 {agg.site_hit}/{agg.site_total} 个可搜站点，共 {agg.items.length} 条
              {agg.site_skipped > 0 ? `（${agg.site_skipped} 个 jar/drpy2 站点已跳过）` : ""}
            </span>
            <button className="btn ghost" onClick={() => setAgg(null)}>
              收起
            </button>
          </div>
          <div className="agg-list">
            {agg.items.map((it) => (
              <div
                key={`${it.source_id}:${it.site_key}:${it.vod_id}`}
                className="agg-item"
                onClick={() => {
                  setSourceId(it.source_id);
                  setSiteKey(it.site_key);
                  openDetail(it.vod_id, it.source_id, it.site_key);
                }}
              >
                {it.vod_pic ? (
                  <img src={it.vod_pic} alt="" loading="lazy" />
                ) : (
                  <div className="no-pic">无封面</div>
                )}
                <div className="agg-meta">
                  <div className="agg-name">{it.vod_name}</div>
                  <div className="agg-sub">
                    {it.site_name}
                    {it.vod_remarks ? ` · ${it.vod_remarks}` : ""}
                  </div>
                </div>
              </div>
            ))}
          </div>
          {agg.items.length === 0 && <div className="empty">没有站点命中，换个关键词试试</div>}
          {agg.failures.length > 0 && (
            <details className="agg-fail">
              <summary>{agg.failures.length} 个站点失败/超时</summary>
              <ul>
                {agg.failures.slice(0, 10).map((f) => (
                  <li key={f.site_key}>
                    {f.site_name}: {f.reason}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}

      {searchResult && (
        <div className="cat-bar">
          <span className="badge">搜索「{keyword}」{searchResult.length} 条</span>
          <button className="btn ghost" onClick={() => setSearchResult(null)}>
            清除
          </button>
        </div>
      )}

      {!searchResult && !agg && cats.length > 0 && (
        <div className="cat-bar">
          {cats.map((c) => (
            <button
              key={c.type_id}
              className={`chip${c.type_id === catId ? " active" : ""}`}
              onClick={() => {
                setCatId(c.type_id);
                setPg(1);
              }}
            >
              {c.type_name}
            </button>
          ))}
        </div>
      )}

      {loading && <div className="empty">加载中…</div>}
      {parsing && <div className="empty">正在解析播放地址…（分享页需多试几秒）</div>}
      {error && <div className="msg err">{error}</div>}

      {!agg && <div className="vod-grid">
        {shown.map((v) => (
          <div className="vod-card" key={v.vod_id} onClick={() => openDetail(v.vod_id)}>
            <div className="vod-pic">
              {v.vod_pic ? (
                <img src={v.vod_pic} alt={v.vod_name} loading="lazy" />
              ) : (
                <div className="no-pic">无封面</div>
              )}
              {v.vod_remarks && <span className="remarks">{v.vod_remarks}</span>}
            </div>
            <div className="vod-name" title={v.vod_name}>
              {v.vod_name}
            </div>
          </div>
        ))}
      </div>}
      {!agg && shown.length === 0 && !loading && !searchResult && cats.length > 0 && (
        <div className="empty">该分类暂无内容</div>
      )}

      {!searchResult && pageCount > 1 && (
        <div className="pager">
          <button className="btn" disabled={pg <= 1} onClick={() => setPg(pg - 1)}>
            上一页
          </button>
          <span>
            {pg} / {pageCount}
          </span>
          <button
            className="btn"
            disabled={pg >= pageCount}
            onClick={() => setPg(pg + 1)}
          >
            下一页
          </button>
        </div>
      )}

      {detail && (
        <div className="modal-mask" onClick={() => setDetail(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <button className="btn ghost close" onClick={() => setDetail(null)}>
              ✕
            </button>
            <div className="detail-head">
              {detail.vod_pic && (
                <img className="detail-pic" src={detail.vod_pic} alt="" />
              )}
              <div>
                <h3>{detail.vod_name}</h3>
                <p className="hint">
                  {[detail.vod_type, detail.vod_year, detail.vod_area]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
                {detail.vod_director && (
                  <p className="hint">导演：{detail.vod_director}</p>
                )}
                {detail.vod_actor && <p className="hint">主演：{detail.vod_actor}</p>}
                {detail.vod_content && (
                  <p className="detail-content">{detail.vod_content}</p>
                )}
              </div>
            </div>
            {detail.play_sources.map((ps) => (
              <div key={ps.from} className="episode-block">
                <div className="episode-from">{ps.from}</div>
                <div className="episode-list">
                  {ps.episodes.map(([name, url]) => (
                    <button
                      key={ps.from + url}
                      className="chip"
                      onClick={() => playEpisode(name, url)}
                    >
                      {name}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {playing && (
        <Player
          url={playing.url}
          kind="hls"
          title={playing.title}
          onClose={() => setPlaying(null)}
        />
      )}
    </div>
  );
}
