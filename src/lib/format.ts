export function fmtTime(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "0:00";
  const s = Math.floor(secs % 60);
  const m = Math.floor((secs / 60) % 60);
  const h = Math.floor(secs / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

export function fmtDuration(secs: number): string {
  if (!secs) return "未知时长";
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins} 分钟`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m ? `${h} 小时 ${m} 分` : `${h} 小时`;
}

export function fmtSize(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = bytes;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export function fmtRelative(unix: number): string {
  if (!unix) return "";
  const diff = Date.now() / 1000 - unix;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  if (diff < 86400 * 7) return `${Math.floor(diff / 86400)} 天前`;
  const d = new Date(unix * 1000);
  return `${d.getMonth() + 1} 月 ${d.getDate()} 日`;
}

export function epLabel(item: { episode: number | null; season: number | null }): string {
  if (item.episode == null) return "正片";
  const ep = String(item.episode).padStart(2, "0");
  if (item.season != null) return `S${String(item.season).padStart(2, "0")}E${ep}`;
  return `第 ${item.episode} 集`;
}

export function progressPct(pos: number, dur: number): number {
  if (!dur || dur <= 0) return 0;
  return Math.min(100, Math.max(0, (pos / dur) * 100));
}
