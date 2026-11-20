import { useCallback, useEffect, useRef, useState } from "react";
import { api, type OnlinePlayback, type Status } from "./lib/api";
import { fmtTime } from "./lib/format";
import { Icon, type IconName } from "./components/Icon";
import { PlayerScreen } from "./components/PlayerScreen";
import { Movies } from "./pages/Movies";
import { Shorts } from "./pages/Shorts";
import { Music } from "./pages/Music";
import { Files } from "./pages/Files";
import { Settings } from "./pages/Settings";
import { Detail } from "./pages/Detail";
import { Live } from "./pages/Live";
import { Online } from "./pages/Online";
import { Sources } from "./pages/Sources";

type View = "movies" | "online" | "live" | "shorts" | "music" | "files" | "sources" | "settings";

const NAV: { key: View; label: string; icon: IconName }[] = [
  { key: "movies", label: "影视", icon: "film" },
  { key: "online", label: "在线点播", icon: "cloud" },
  { key: "live", label: "直播", icon: "live" },
  { key: "shorts", label: "短剧", icon: "shorts" },
  { key: "music", label: "音乐", icon: "music" },
  { key: "files", label: "本地文件", icon: "folder" },
  { key: "sources", label: "订阅源", icon: "spark" },
  { key: "settings", label: "设置", icon: "settings" },
];

const TITLES: Record<View, { title: string; sub: string }> = {
  movies: { title: "影视库", sub: "本地电影与剧集，续播位置自动保存" },
  online: { title: "在线点播", sub: "TVBox 站点跨站搜索，点开即播" },
  live: { title: "直播", sub: "IPTV 频道，按分组浏览" },
  shorts: { title: "短剧", sub: "竖屏内容，播完自动接下一集" },
  music: { title: "音乐", sub: "专辑与无损曲库" },
  files: { title: "本地文件", sub: "管理媒体目录与扫描" },
  sources: { title: "订阅源", sub: "IPTV 与 TVBox 订阅管理" },
  settings: { title: "设置", sub: "内核与播放行为" },
};

type Playing = {
  kind: "video" | "audio" | "live" | "online";
  title: string;
  itemId?: number;
  online?: OnlinePlayback;
};

function initialRoute(): { view: View; series: number | null } {
  const hash = window.location.hash.replace(/^#/, "");
  if (hash.startsWith("series=")) {
    const id = Number(hash.slice(7));
    return { view: "movies", series: Number.isFinite(id) ? id : null };
  }
  const hit = NAV.find((n) => n.key === hash);
  return { view: hit ? hit.key : "movies", series: null };
}

export default function App() {
  const boot = initialRoute();
  const [view, setView] = useState<View>(boot.view);
  const [detailId, setDetailId] = useState<number | null>(boot.series);
  const [playing, setPlaying] = useState<Playing | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [query, setQuery] = useState("");
  const [toastMsg, setToastMsg] = useState("");
  const [urlOpen, setUrlOpen] = useState(false);
  const [urlInput, setUrlInput] = useState("");
  const [urlUa, setUrlUa] = useState(() => localStorage.getItem("xv.direct.ua") ?? "");
  const [sourceStamp, setSourceStamp] = useState(0);
  const toastTimer = useRef(0);
  const statusBusy = useRef(false);

  const toast = useCallback((msg: string) => {
    setToastMsg(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToastMsg(""), 2800);
  }, []);

  const loadStatus = useCallback(() => {
    if (statusBusy.current) return;
    statusBusy.current = true;
    api
      .status()
      .then((s) => {
        // 归一化：后端若返回不完整对象（缺 engine），按未就绪处理，
        // 避免下游 status?.engine.xxx 在 status 为真值但无 engine 时崩溃
        setStatus(s && s.engine ? s : null);
      })
      .catch(() => {})
      .finally(() => {
        statusBusy.current = false;
      });
  }, []);

  useEffect(() => {
    loadStatus();
    const t = window.setInterval(loadStatus, 4000);
    return () => window.clearInterval(t);
  }, [loadStatus]);

  const play = useCallback(
    async (itemId: number, title: string, kind: "video" | "audio") => {
      try {
        await api.open({ item_id: itemId });
        setPlaying({ itemId, title, kind });
      } catch (e) {
        toast(`播放失败：${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [toast]
  );

  const playLive = useCallback(
    async (ch: { url: string; name: string; urls?: string[]; headers?: string }) => {
      try {
        await api.openLive({
          url: ch.url,
          urls: ch.urls,
          name: ch.name,
          headers: ch.headers ?? "",
        });
        setPlaying({ kind: "live", title: ch.name });
      } catch (e) {
        toast(`播放失败：${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [toast]
  );

  /** 打开在线点播的某一集：站点线路走解析，site 为空时按直链处理。 */
  const openEpisode = useCallback(
    async (online: OnlinePlayback, index: number): Promise<boolean> => {
      const ep = online.episodes[index];
      if (!ep) return false;
      try {
        const r = online.site
          ? await api.vodOpen({ site: online.site, url: ep.url, flag: online.flag })
          : await api.openUrl({ url: ep.url });
        const next = { ...online, index };
        setPlaying({
          kind: "online",
          title: ep.name ? `${online.title} · ${ep.name}` : online.title,
          online: next,
        });
        if (r.resume > 0) toast(`已从 ${fmtTime(r.resume)} 继续播放`);
        return true;
      } catch (e) {
        toast(`播放失败：${e instanceof Error ? e.message : String(e)}`);
        return false;
      }
    },
    [toast]
  );

  const playOnline = useCallback(
    (p: OnlinePlayback) => openEpisode(p, p.index),
    [openEpisode]
  );

  const playDirectUrl = useCallback(async () => {
    const url = urlInput.trim();
    if (!url) return;
    const ua = urlUa.trim();
    const headers = ua ? `User-Agent: ${ua.replace(/,/g, " ")}` : "";
    try {
      const r = await api.openUrl({ url, headers });
      localStorage.setItem("xv.direct.ua", ua);
      setPlaying({
        kind: "online",
        title: url,
        online: {
          title: url,
          site: "",
          flag: "",
          episodes: [{ name: "在线播放", url }],
          index: 0,
        },
      });
      setUrlOpen(false);
      setUrlInput("");
      if (r.resume > 0) toast(`已从 ${fmtTime(r.resume)} 继续播放`);
    } catch (e) {
      toast(`播放失败：${e instanceof Error ? e.message : String(e)}`);
    }
  }, [urlInput, urlUa, toast]);

  const go = (v: View) => {
    setView(v);
    setDetailId(null);
    setQuery("");
    window.location.hash = v;
  };

  const meta = TITLES[view];
  const canSearch = view === "movies" || view === "shorts" || view === "music";
  const counts = status?.counts;

  return (
    <div className="shell">
      <aside className="side">
        <div className="brand">
          <div className="brand-mark">
            <svg viewBox="0 0 24 24" fill="none">
              <path d="M5 4.5v15l12-7.5z" fill="#0b0f1a" />
            </svg>
          </div>
          <div className="brand-text">
            <div className="brand-name">X-Video</div>
            <div className="brand-sub">桌面视频播放器</div>
          </div>
        </div>

        {NAV.map((n) => {
          const count =
            n.key === "movies"
              ? (counts?.movie ?? 0) + (counts?.tv ?? 0)
              : n.key === "shorts"
                ? counts?.short
                : n.key === "music"
                  ? counts?.music
                  : undefined;
          return (
            <button
              key={n.key}
              className={`nav-item ${view === n.key && !detailId ? "on" : ""}`}
              onClick={() => go(n.key)}
            >
              <Icon name={n.icon} />
              <span className="nav-label">{n.label}</span>
              {count != null && count > 0 && <span className="nav-count">{count}</span>}
            </button>
          );
        })}

        <div className="side-foot">
          <div className="engine-chip">
            <i
              className={`dot ${
                status?.engine?.available ? "ok" : status?.engine ? "err" : ""
              }`}
            />
            <span>
              {status?.engine
                ? status.engine.available
                  ? "内核就绪 · libmpv"
                  : "内核不可用"
                : "检测内核…"}
            </span>
          </div>
        </div>
      </aside>

      <main className="main">
        <div className="topbar">
          <div>
            <h1>{detailId ? "剧集详情" : meta.title}</h1>
            <div className="sub">{detailId ? "选择集数或继续上次进度" : meta.sub}</div>
          </div>
          {canSearch && !detailId && (
            <div className="search">
              <Icon name="search" size={15} />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索片名、专辑"
                spellCheck={false}
              />
            </div>
          )}
          <button className="icon-btn" onClick={loadStatus} title="刷新状态">
            <Icon name="refresh" />
          </button>
          <button
            className="icon-btn"
            onClick={() => setUrlOpen(true)}
            title="打开网络链接"
          >
            <Icon name="link" />
          </button>
          <button className="icon-btn" onClick={() => go("files")} title="添加媒体目录">
            <Icon name="plus" />
          </button>
        </div>

        <div className="scroll">
          {detailId ? (
            <Detail seriesId={detailId} onBack={() => setDetailId(null)} onPlay={play} />
          ) : view === "movies" ? (
            <Movies
              status={status}
              query={query}
              onOpenSeries={setDetailId}
              onPlay={(id, title) => play(id, title, "video")}
            />
          ) : view === "online" ? (
            <Online
              toast={toast}
              onGoSources={() => go("sources")}
              onPlay={playOnline}
            />
          ) : view === "live" ? (
            <Live
              key={sourceStamp}
              toast={toast}
              onGoSources={() => go("sources")}
              onPlay={playLive}
            />
          ) : view === "shorts" ? (
            <Shorts query={query} onOpenSeries={setDetailId} />
          ) : view === "music" ? (
            <Music query={query} onOpenSeries={setDetailId} />
          ) : view === "files" ? (
            <Files status={status} onChanged={loadStatus} toast={toast} />
          ) : view === "sources" ? (
            <Sources
              toast={toast}
              onGoLive={() => {
                setSourceStamp((n) => n + 1);
                go("live");
              }}
              onChanged={() => {
                loadStatus();
                setSourceStamp((n) => n + 1);
              }}
            />
          ) : (
            <Settings status={status} toast={toast} />
          )}
        </div>
      </main>

      {playing && (
        <PlayerScreen
          itemId={playing.itemId}
          title={playing.title}
          kind={playing.kind}
          online={playing.online}
          onPlayOnline={openEpisode}
          onClose={() => {
            setPlaying(null);
            loadStatus();
          }}
        />
      )}

      {urlOpen && (
        <div className="sheet-mask" onClick={() => setUrlOpen(false)}>
          <div className="sheet url-sheet" onClick={(e) => e.stopPropagation()}>
            <button className="sheet-close" onClick={() => setUrlOpen(false)}>
              <Icon name="close" size={16} />
            </button>
            <h2>打开网络链接</h2>
            <p className="hint">
              支持 HTTP(S) 直链、HLS（m3u8）、RTSP 等地址；再次打开同一地址会从上次位置继续。
            </p>
            <div className="url-row">
              <input
                className="input"
                autoFocus
                value={urlInput}
                onChange={(e) => setUrlInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && playDirectUrl()}
                placeholder="https://example.com/video.m3u8"
                spellCheck={false}
              />
              <button
                className="btn-primary"
                disabled={!urlInput.trim()}
                onClick={playDirectUrl}
              >
                播放
              </button>
            </div>
            <label className="url-field">
              <span>User-Agent（可选）</span>
              <input
                className="input"
                value={urlUa}
                onChange={(e) => setUrlUa(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && playDirectUrl()}
                placeholder="留空使用播放器默认 UA，例如 Mozilla/5.0 ..."
                spellCheck={false}
              />
            </label>
          </div>
        </div>
      )}

      {toastMsg && <div className="toast">{toastMsg}</div>}
    </div>
  );
}
