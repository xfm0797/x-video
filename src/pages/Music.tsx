import { useEffect, useState } from "react";
import { api, type Series } from "../lib/api";
import { PosterCard } from "../components/PosterCard";

type Props = {
  query: string;
  onOpenSeries: (id: number) => void;
};

export function Music({ query, onOpenSeries }: Props) {
  const [rows, setRows] = useState<Series[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const t = window.setTimeout(async () => {
      try {
        const r = await api.series({ kind: "music", q: query || undefined, limit: 400 });
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
        <div className="big">还没有音乐</div>
        <div style={{ maxWidth: 420, lineHeight: 1.9 }}>
          支持 FLAC / APE / MP3 / M4A / WAV / DSD 等格式。
          <br />
          优先读取内嵌标签（专辑、艺术家、封面），没有标签时用文件夹名归类。
        </div>
      </div>
    );

  return (
    <>
      <div className="section-head">
        <h2>专辑</h2>
        <span className="count">{rows.length} 张</span>
      </div>
      <div className="grid music">
        {rows.map((s) => (
          <PosterCard key={s.id} series={s} kind="music" onOpen={() => onOpenSeries(s.id)} />
        ))}
      </div>
    </>
  );
}
