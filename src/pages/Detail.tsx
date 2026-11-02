import { useEffect, useState } from "react";
import { api, art, type Item, type Series } from "../lib/api";
import { epLabel, fmtSize, fmtTime, progressPct } from "../lib/format";
import { Icon } from "../components/Icon";

type Props = {
  seriesId: number;
  onBack: () => void;
  onPlay: (itemId: number, title: string, kind: "video" | "audio") => void;
};

const KIND_LABEL: Record<string, string> = {
  movie: "电影",
  tv: "剧集",
  short: "短剧",
  music: "专辑",
};

export function Detail({ seriesId, onBack, onPlay }: Props) {
  const [series, setSeries] = useState<Series | null>(null);
  const [items, setItems] = useState<Item[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    let alive = true;
    api
      .seriesDetail(seriesId)
      .then((r) => {
        if (!alive) return;
        setSeries(r.series);
        setItems(r.items);
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [seriesId]);

  if (error) return <div className="empty">{error}</div>;
  if (!series) return <div className="empty">正在读取详情…</div>;

  const isMusic = series.kind === "music";
  const next = items.find((i) => !i.finished && i.position > 0) || items[0];
  const play = (it: Item) =>
    onPlay(it.id, `${series.title} · ${epLabel(it)}`, isMusic ? "audio" : "video");

  return (
    <>
      <button className="btn" onClick={onBack} style={{ marginBottom: 16 }}>
        ← 返回列表
      </button>

      <div className="detail-hero">
        <div className="bg" style={{ backgroundImage: `url(${art("series", series.id)})` }} />
        <div className="veil" />
        <div className="detail-poster">
          <img src={art("series", series.id)} alt={series.title} />
        </div>
        <div className="detail-info">
          <h2>{series.title}</h2>
          <div className="detail-tags">
            <span className="pill">{KIND_LABEL[series.kind] || series.kind}</span>
            {series.year ? <span className="pill">{series.year}</span> : null}
            <span className="pill">
              {isMusic ? `${series.total} 首` : `共 ${series.total} 集`}
            </span>
            {series.watched > 0 ? <span className="pill">已看 {series.watched}</span> : null}
          </div>
          <div style={{ display: "flex", gap: 10, marginTop: 6 }}>
            {next && (
              <button className="btn-play" onClick={() => play(next)}>
                <Icon name="play" size={15} />
                {next.position > 0 ? `继续 ${fmtTime(next.position)}` : "开始播放"}
              </button>
            )}
            {next && next.position > 0 && (
              <button
                className="btn-ghost"
                onClick={() => onPlay(next.id, `${series.title} · ${epLabel(next)}`, isMusic ? "audio" : "video")}
              >
                从头播放
              </button>
            )}
          </div>
        </div>
      </div>

      {isMusic ? (
        <div className="panel" style={{ padding: "10px 14px" }}>
          {items.map((it, i) => (
            <button
              key={it.id}
              className="fs-row"
              onClick={() => play(it)}
              style={{ height: 42 }}
            >
              <span style={{ width: 26, color: "var(--dim-2)" }}>{i + 1}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {it.name}
              </span>
              <span style={{ color: "var(--dim-2)", fontSize: 12 }}>{fmtTime(it.duration)}</span>
            </button>
          ))}
        </div>
      ) : (
        <section className="section">
          <div className="section-head">
            <h2>选集</h2>
            <span className="count">{items.length} 集</span>
          </div>
          <div className="eps">
            {items.map((it, i) => {
              const pct = progressPct(it.position, it.duration);
              return (
                <button key={it.id} className="ep" onClick={() => play(it)} title={it.name}>
                  {it.season != null
                    ? `S${String(it.season).padStart(2, "0")}E${String(it.episode ?? i + 1).padStart(2, "0")}`
                    : String(it.episode ?? i + 1).padStart(2, "0")}
                  {it.finished && <i className="seen" />}
                  {pct > 0 && !it.finished && (
                    <span className="prog">
                      <i style={{ width: `${pct}%` }} />
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          <div style={{ marginTop: 18, display: "flex", gap: 16, color: "var(--dim-2)", fontSize: 12 }}>
            <span>总大小 {fmtSize(items.reduce((a, b) => a + b.size, 0))}</span>
            {items[0]?.width ? <span>{items[0].width}×{items[0].height}</span> : null}
            {items[0]?.vcodec ? <span>{items[0].vcodec}</span> : null}
          </div>
        </section>
      )}
    </>
  );
}
