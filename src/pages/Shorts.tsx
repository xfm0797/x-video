import { useEffect, useState } from "react";
import { api, type Series } from "../lib/api";
import { PosterCard } from "../components/PosterCard";

type Props = {
  query: string;
  onOpenSeries: (id: number) => void;
};

export function Shorts({ query, onOpenSeries }: Props) {
  const [rows, setRows] = useState<Series[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const t = window.setTimeout(async () => {
      try {
        const r = await api.series({ kind: "short", q: query || undefined, limit: 300 });
        if (alive) setRows(r.items);
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
  }, [query]);

  if (loading && rows.length === 0) return <div className="empty">正在读取…</div>;

  if (rows.length === 0)
    return (
      <div className="empty">
        <div className="big">还没有识别到短剧</div>
        <div style={{ maxWidth: 420, lineHeight: 1.9 }}>
          竖屏视频（宽高比小于 0.8）会被自动标记为短剧；集数达到 60 集以上
          的剧集也会先按短剧处理，读取分辨率后自动纠正。
        </div>
      </div>
    );

  return (
    <>
      <div className="section-head">
        <h2>竖屏短剧</h2>
        <span className="count">{rows.length} 部</span>
        <span className="more">点击进入后可直接从任意一集开始，播完自动接下一集</span>
      </div>
      <div className="grid shorts">
        {rows.map((s) => (
          <PosterCard key={s.id} series={s} kind="shorts" onOpen={() => onOpenSeries(s.id)} />
        ))}
      </div>
    </>
  );
}
