export type Series = {
  id: number;
  title: string;
  kind: "movie" | "tv" | "short" | "music" | string;
  year: number | null;
  total: number;
  added_at: number;
  updated_at: number;
  watched: number;
  last_at: number;
};

export type Item = {
  id: number;
  series_id: number | null;
  name: string;
  path: string;
  episode: number | null;
  season: number | null;
  duration: number;
  width: number;
  height: number;
  vertical: boolean;
  size: number;
  added_at: number;
  position: number;
  finished: boolean;
  vcodec: string;
  acodec: string;
  container: string;
};

export type ContinueRow = {
  item_id: number;
  series_id: number | null;
  series_title: string;
  name: string;
  episode: number | null;
  position: number;
  duration: number;
  updated_at: number;
  vertical: boolean;
};

export type Track = {
  id: number;
  lang: string;
  title: string;
  codec: string;
  selected: boolean;
  channels: string;
};

export type EngineState = {
  available: boolean;
  loaded: boolean;
  playing: boolean;
  position: number;
  duration: number;
  volume: number;
  muted: boolean;
  speed: number;
  title: string;
  path: string;
  width: number;
  height: number;
  vcodec: string;
  acodec: string;
  container: string;
  hwdec: string;
  audio_tracks: Track[];
  sub_tracks: Track[];
  audio_id: number | null;
  sub_id: number | null;
  eof: boolean;
  error: string | null;
};

export type Session = {
  item_id: number | null;
  series_id: number | null;
  queue: number[];
  index: number;
  mode: string;
};

export type PlaybackState = { engine: EngineState; session: Session };

export type ScanReport = {
  scanned: number;
  added: number;
  updated: number;
  skipped: number;
  errors: number;
  removed: number;
  elapsed_ms: number;
};

export type Status = {
  version: string;
  engine: { available: boolean; lib: string; error: string | null };
  counts: { movie: number; tv: number; short: number; music: number };
  scanning: boolean;
  last_scan: ScanReport | null;
  data_dir: string;
  uptime: number;
};

export type PlayerPrefs = {
  volume: number;
  speed: number;
  mode: string;
  hwdec: string;
};

export type Sub = {
  id: string;
  name: string;
  url: string;
  kind: "iptv" | "tvbox" | string;
  enabled: boolean;
  added_at: number;
  updated_at: number;
  error: string | null;
  summary: string;
  groups: number;
  channels: number;
  sites: number;
};

export type LiveChannel = {
  name: string;
  url: string;
  logo?: string | null;
  tvg_id?: string | null;
  headers?: string | null;
};

export type LiveGroup = {
  sub_id: string;
  sub_name: string;
  group: string;
  channels: LiveChannel[];
};

export type VodSite = {
  sub_id: string;
  sub_name: string;
  key: string;
  name: string;
  kind: number;
  supported: boolean;
  searchable: boolean;
  /** 需要 Java spider 内核（csp_*），跑不了 */
  needs_java: boolean;
};

export type VodHit = {
  site: string;
  sub_name: string;
  site_name: string;
  id: string;
  name: string;
  pic: string | null;
  remarks: string | null;
  year: string | null;
};

export type VodEpisode = { name: string; url: string };
export type VodSource = { from: string; episodes: VodEpisode[] };

export type VodDetail = {
  id: string;
  name: string;
  pic: string | null;
  year: string | null;
  area: string | null;
  kind: string | null;
  actor: string | null;
  director: string | null;
  content: string | null;
  sources: VodSource[];
};

const BASE = "";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, init);
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) {
    const msg =
      data && typeof data === "object" && "error" in (data as Record<string, unknown>)
        ? String((data as Record<string, unknown>).error)
        : res.statusText;
    throw new Error(msg || `请求失败（${res.status}）`);
  }
  return data as T;
}

const get = <T,>(path: string) => request<T>(path);
const post = <T,>(path: string, body: unknown) =>
  request<T>(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });

export const api = {
  status: () => get<Status>("/api/status"),
  series: (params: {
    kind?: string;
    q?: string;
    sort?: string;
    limit?: number;
    offset?: number;
  }) => {
    const q = new URLSearchParams();
    if (params.kind) q.set("kind", params.kind);
    if (params.q) q.set("q", params.q);
    if (params.sort) q.set("sort", params.sort);
    if (params.limit) q.set("limit", String(params.limit));
    if (params.offset) q.set("offset", String(params.offset));
    return get<{ items: Series[]; total: number }>(`/api/library/series?${q}`);
  },
  seriesDetail: (id: number) =>
    get<{ series: Series; items: Item[] }>(`/api/library/series/${id}`),
  cont: () => get<{ items: ContinueRow[] }>("/api/library/continue"),
  recent: () => get<{ items: Item[] }>("/api/library/recent"),
  roots: () => get<{ items: string[] }>("/api/library/roots"),
  addRoot: (path: string) => post<{ ok: boolean }>("/api/library/roots", { path }),
  removeRoot: (path: string) =>
    post<{ removed: number }>("/api/library/roots/remove", { path }),
  scan: (path: string) => post<{ started: boolean }>("/api/library/scan", { path }),
  queue: (itemId: number) => get<{ items: Item[] }>(`/api/library/item/${itemId}/queue`),
  state: () => get<PlaybackState>("/api/player/state"),
  open: (body: { item_id?: number; path?: string; start?: number }) =>
    post<{ ok: boolean }>("/api/player/open", body),
  cmd: (cmd: string, opts: { value?: number; text?: string; id?: number } = {}) =>
    post<{ ok: boolean }>("/api/player/cmd", { cmd, ...opts }),
  rect: (r: { x: number; y: number; w: number; h: number; visible: boolean }) =>
    post<{ ok: boolean }>("/api/player/rect", r),
  settings: () => get<{ player: PlayerPrefs; roots: string[] }>("/api/settings"),
  saveSettings: (player: PlayerPrefs) =>
    post<{ ok: boolean }>("/api/settings", { player }),
  drives: () => get<{ items: { name: string; path: string }[] }>("/api/fs/drives"),
  ls: (path: string) =>
    get<{ path: string; parent: string; items: { name: string; path: string }[] }>(
      `/api/fs/list?path=${encodeURIComponent(path)}`
    ),
  fullscreen: (on: boolean) => post<{ ok: boolean }>("/api/window/fullscreen", { on }),

  // ---- 在线源 ----
  sources: () => get<{ items: Sub[] }>("/api/sources"),
  addSource: (body: { name?: string; url: string; kind?: string }) =>
    post<{ sub: Sub; items: Sub[] }>("/api/sources", body),
  removeSource: (id: string) => post<{ items: Sub[] }>("/api/sources/remove", { id }),
  refreshSources: (id = "") => post<{ items: Sub[] }>("/api/sources/refresh", { id }),
  toggleSource: (id: string, enabled: boolean) =>
    post<{ items: Sub[] }>("/api/sources/toggle", { id, enabled }),
  live: (sub?: string) =>
    get<{ groups: LiveGroup[] }>(`/api/live${sub ? `?sub=${encodeURIComponent(sub)}` : ""}`),
  openLive: (body: { url: string; name: string; headers?: string }) =>
    post<{ ok: boolean }>("/api/live/open", body),
  vodSites: () => get<{ items: VodSite[] }>("/api/vod/sites"),
  vodSearch: (q: string, site?: string, limit?: number) => {
    const p = new URLSearchParams({ q });
    if (site) p.set("site", site);
    if (limit) p.set("limit", String(limit));
    return get<{ hits: VodHit[]; failed: { site: string; error: string }[] }>(
      `/api/vod/search?${p}`
    );
  },
  vodDetail: (site: string, id: string) =>
    get<{ detail: VodDetail }>(
      `/api/vod/detail?site=${encodeURIComponent(site)}&id=${encodeURIComponent(id)}`
    ),
  vodOpen: (body: { site: string; url: string; flag: string }) =>
    post<{ ok: boolean; url: string }>("/api/vod/open", body),
};

export const art = (kind: "item" | "series", id: number) => `/api/artwork/${kind}/${id}`;
