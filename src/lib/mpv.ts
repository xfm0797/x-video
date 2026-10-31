// mpv 内核（libmpv）控制接口
import { api } from "./api";

export type MpvMode = "embed" | "floating";

export interface MpvRect {
  x: number;
  y: number;
  w: number;
  h: number;
  scale: number;
  sx: number;
  sy: number;
}

export interface MpvStatus {
  available: boolean;
  path?: string | null;
  version?: string | null;
  source?: string | null;
  platform: string;
  can_install: boolean;
  install_dir: string;
  installing: boolean;
  progress: number;
  running: boolean;
  idle: boolean;
  paused: boolean;
  position: number;
  duration: number;
  volume: number;
  mute: boolean;
  eof: boolean;
  title: string;
  error?: string | null;
}

export const mpv = {
  status: () => api.get<MpvStatus>("/api/mpv/status"),
  probe: () => api.post<MpvStatus>("/api/mpv/probe"),
  install: () => api.post<{ path: string; version: string }>("/api/mpv/install"),
  setPath: (path: string) => api.post<MpvStatus>("/api/mpv/path", { path }),
  open: (body: { url: string; title: string; mode: MpvMode; rect: MpvRect }) =>
    api.post<{ ok: boolean }>("/api/mpv/open", body),
  rect: (rect: MpvRect) => api.post<{ ok: boolean }>("/api/mpv/rect", { rect }),
  cmd: (args: unknown[]) => api.post<{ ok: boolean }>("/api/mpv/cmd", { args }),
  close: () => api.post<{ ok: boolean }>("/api/mpv/close"),
};

const MODE_KEY = "xv-mpv-mode";

export function loadMode(): MpvMode {
  return localStorage.getItem(MODE_KEY) === "floating" ? "floating" : "embed";
}

export function saveMode(mode: MpvMode) {
  localStorage.setItem(MODE_KEY, mode);
}

/** 浏览器内核完全吃不了、必须交给 mpv 的容器 */
export function needsMpv(kind: string): boolean {
  return kind === "flv";
}

/** 量取元素在窗口客户区与屏幕上的位置（物理像素） */
export function measureRect(el: HTMLElement): MpvRect {
  const r = el.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const borderX = Math.max(0, (window.outerWidth - window.innerWidth) / 2);
  const chromeY = Math.max(0, window.outerHeight - window.innerHeight - borderX);
  return {
    x: Math.round(r.left * dpr),
    y: Math.round(r.top * dpr),
    w: Math.round(r.width * dpr),
    h: Math.round(r.height * dpr),
    scale: dpr,
    sx: Math.round((window.screenX + borderX + r.left) * dpr),
    sy: Math.round((window.screenY + chromeY + r.top) * dpr),
  };
}

export function formatTime(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "00:00";
  const total = Math.floor(sec);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
