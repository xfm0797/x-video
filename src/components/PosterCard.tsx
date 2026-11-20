import { useState } from "react";
import { art, type Series } from "../lib/api";
import { Icon } from "./Icon";

type Props = {
  series: Series;
  kind: "movies" | "shorts" | "music";
  onOpen: () => void;
};

export function PosterCard({ series, kind, onOpen }: Props) {
  const [broken, setBroken] = useState(false);
  const pct = series.total > 0 ? (series.watched / series.total) * 100 : 0;

  return (
    <button className={`card ${kind}`} onClick={onOpen} title={series.title}>
      <div className="card-pic">
        {broken ? (
          <div className="poster-fallback">{series.title.slice(0, 1)}</div>
        ) : (
          <img
            src={art("series", series.id)}
            alt={series.title}
            loading="lazy"
            onError={() => setBroken(true)}
          />
        )}
        <div className="card-play">
          <span>
            <Icon name="play" size={16} />
          </span>
        </div>
        {series.year ? <div className="badge-new">{series.year}</div> : null}
        {pct > 0 && (
          <div className="badge-progress">
            <i style={{ width: `${pct}%` }} />
          </div>
        )}
      </div>
      <div className="card-name">{series.title}</div>
      <div className="card-sub">
        <span>
          {kind === "music"
            ? `${series.total} 首`
            : series.total > 1
              ? `共 ${series.total} ${kind === "shorts" ? "集" : "集"}`
              : "电影"}
        </span>
        {series.watched > 0 && <span>· 已看 {series.watched}</span>}
      </div>
    </button>
  );
}

/** 网格加载占位：复用卡片的尺寸与 .skel 微光动画。 */
export function SkeletonCards({
  count = 12,
  kind = "movies",
}: {
  count?: number;
  kind?: "movies" | "shorts" | "music";
}) {
  return (
    <>
      {Array.from({ length: count }).map((_, i) => (
        <div className={`card ${kind}`} key={i} aria-hidden="true">
          <div className="card-pic skel" />
          <div className="skel-line" style={{ width: "72%" }} />
          <div className="skel-line" style={{ width: "46%" }} />
        </div>
      ))}
    </>
  );
}
