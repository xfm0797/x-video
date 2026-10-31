// API 封装：vite 开发态(5173)与内嵌服务(21600)同源；
// 其余场景（如 Tauri 资源协议 tauri://）直连本地服务。
const SAME_ORIGIN_PORTS = ["21600", "5173"];
export const API_BASE = SAME_ORIGIN_PORTS.includes(location.port)
  ? ""
  : "http://127.0.0.1:21600";

export class ApiError extends Error {}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`${API_BASE}${path}`, init);
  const text = await resp.text();
  let data: unknown = null;
  try {
    data = JSON.parse(text);
  } catch {
    /* 非 JSON 响应 */
  }
  if (!resp.ok) {
    const msg =
      (data as { error?: string } | null)?.error ?? `请求失败 (${resp.status})`;
    throw new ApiError(msg);
  }
  return data as T;
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
    }),
  del: <T>(path: string) => request<T>(path, { method: "DELETE" }),
};

// ---------- 类型 ----------

export type SourceKind = "m3u" | "txt" | "tvbox";

export interface Source {
  id: string;
  name: string;
  type: SourceKind;
  url: string;
  enabled: boolean;
  added_at: number;
  channel_count: number;
  site_count: number;
  last_error: string | null;
}

export interface LiveChannel {
  name: string;
  url: string;
  logo?: string;
  tvg_id?: string;
}

export interface LiveGroup {
  name: string;
  channels: LiveChannel[];
}

export interface TvBoxSite {
  key: string;
  name: string;
  site_type: number;
  api: string;
  searchable?: number;
}

export interface VodCategory {
  type_id: string;
  type_name: string;
}

export interface VodItem {
  vod_id: string;
  vod_name: string;
  vod_pic?: string;
  vod_remarks?: string;
}

export interface PlaySource {
  from: string;
  episodes: [string, string][];
}

export interface VodDetail {
  vod_id: string;
  vod_name: string;
  vod_pic?: string;
  vod_year?: string;
  vod_area?: string;
  vod_type?: string;
  vod_actor?: string;
  vod_director?: string;
  vod_content?: string;
  play_sources: PlaySource[];
}

export interface PlayUrl {
  url: string;
  kind: string;
  raw: string;
}

export const KIND_LABEL: Record<SourceKind, string> = {
  m3u: "M3U 直播",
  txt: "TXT 直播",
  tvbox: "TVBox",
};
