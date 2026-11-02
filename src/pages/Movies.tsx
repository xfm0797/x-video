import { useEffect, useState } from "react";
import { api, art, type ContinueRow, type Series, type Status } from "../lib/api";
import { fmtRelative, fmtTime, progressPct } from "../lib/format";
import { Icon } from "../components/Icon";
import { PosterCard } from "../components/PosterCard";

type Props = {
  status: Status | null;
  query: string;
  onOpenSeries: (id: number) => void;
  onPlay: (itemId: number, title: string) => void;
};

const TABS = [
  { key: "all", label: "全部" },
  { key: "movie", label: "电影" },
  { key: "tv", label: "剧集" },
] as const;

export function Movies({ status, query, onOpenSeries, onPlay }: Props) {
  const [tab, setTab] = useState<string>("all");
  const [sort, setSort] = useState("recent");
  const [rows, setRows] = useState<Series[]>([]);
  const [cont, setCont] = useState<ContinueRow[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const t = window.setTimeout(async () => {
      try {
        const kind = tab === "all" ? undefined : tab;
        const [list, c] = await Promise.all([
          api.series({ kind, q: query || undefined, sort, limit: 200 }),
          api.cont(),
        ]);
        if (!alive) return;
        setRows(list.items);
        setCont(c.items);
      } catch {
        if (alive) setRows([]);
      } finally {
        if (alive) setLoading(false);
      }
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [tab, sort, query]);

  const hero = cont[0];
  const heroSeries = hero
    ? rows.find((r) => r.id === hero.series_id) || null
    : rows[0] || null;
  const heroTitle = hero ? hero.series_title : heroSeries?.title || "";
  const heroImg = hero
    ? art("item", hero.item_id)
    : heroSeries
      ? art("series", heroSeries.id)
      : "";

  return (
    <>
      {heroTitle && (
        <div className="hero" onClick={() => hero && onPlay(hero.item_id, hero.series_title)}>
          <div className="hero-bg" style={{ backgroundImage: `url(${heroImg})` }} />
          <div className="hero-shade" />
          <div className="hero-body">
            <div className="hero-text">
              <div className="hero-kicker">
                <Icon name="spark" size={13} />
                {hero ? "继续观看" : "刚刚入库"}
              </div>
              <h1 className="hero-title">{heroTitle}</h1>
              <div className="hero-meta">
                {hero ? (
                  <>
                    <b>{hero.episode ? `第 ${hero.episode} 集` : "正片"}</b>
                    <span>
                      已看 {fmtTime(hero.position)} / {fmtTime(hero.duration)}
                    </span>
                    <span>{fmtRelative(hero.updated_at)}</span>
                  </>
                ) : (
                  <>
                    {heroSeries?.year && <span>{heroSeries.year}</span>}
                    <span>{heroSeries?.total} 集</span>
                    {heroSeries?.watched ? <span>已看 {heroSeries.watched} 集</span> : null}
                  </>
                )}
              </div>
              <div className="hero-actions">
                <button
                  className="btn-play"
                  onClick={(e) => {
                    e.stopPropagation();
                    if (hero) onPlay(hero.item_id, hero.series_title);
                    else if (heroSeries) onOpenSeries(heroSeries.id);
                  }}
                >
                  <Icon name="play" size={15} />
                  {hero ? `继续播放 ${fmtTime(hero.position)}` : "开始观看"}
                </button>
                {heroSeries && (
                  <button
                    className="btn-ghost"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpenSeries(heroSeries.id);
                    }}
                  >
                    详情与选集
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {cont.length > 1 && (
        <section className="section">
          <div className="section-head">
            <h2>继续观看</h2>
            <span className="count">{cont.length} 部未看完</span>
          </div>
          <div className="row">
            {cont.slice(0, 12).map((c) => (
              <button
                key={c.item_id}
                className="wide"
                onClick={() => onPlay(c.item_id, c.series_title)}
              >
                <div className="wide-pic">
                  <img src={art("item", c.item_id)} alt="" loading="lazy" />
                </div>
                <div className="wide-info">
                  <div className="wide-title">{c.series_title}</div>
                  <div className="wide-sub">
                    <span>{c.episode ? `第 ${c.episode} 集` : "正片"}</span>
                    <span>{Math.round(progressPct(c.position, c.duration))}%</span>
                  </div>
                  <div className="wide-bar">
                    <i style={{ width: `${progressPct(c.position, c.duration)}%` }} />
                  </div>
                </div>
              </button>
            ))}
          </div>
        </section>
      )}

      <section className="section">
        <div className="chips">
          {TABS.map((t) => (
            <button
              key={t.key}
              className={`chip ${tab === t.key ? "on" : ""}`}
              onClick={() => setTab(t.key)}
            >
              {t.label}
            </button>
          ))}
          <button
            className={`chip ${sort === "added" ? "on" : ""}`}
            onClick={() => setSort(sort === "added" ? "recent" : "added")}
            style={{ marginLeft: "auto" }}
          >
            最近入库
          </button>
        </div>

        {loading && rows.length === 0 ? (
          <div className="empty">正在读取媒体库…</div>
        ) : rows.length === 0 ? (
          <div className="empty">
            <div className="big">媒体库还是空的</div>
            <div>
              到「本地文件」里选择一个文件夹，扫描后视频会自动归集成影视与剧集。
            </div>
          </div>
        ) : (
          <div className="grid movies">
            {rows.map((s) => (
              <PosterCard
                key={s.id}
                series={s}
                kind="movies"
                onOpen={() => onOpenSeries(s.id)}
              />
            ))}
          </div>
        )}
      </section>

      {status && status.counts.movie + status.counts.tv + status.counts.short > 0 && (
        <div style={{ color: "var(--dim-2)", fontSize: 12, paddingBottom: 12 }}>
          共 {status.counts.movie} 部电影 · {status.counts.tv} 部剧集 ·{" "}
          {status.counts.short} 部短剧
        </div>
      )}
    </>
  );
}
