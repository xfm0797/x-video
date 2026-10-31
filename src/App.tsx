import { useEffect, useState } from "react";
import { api, type Source } from "./lib/api";
import Sources from "./pages/Sources";
import Live from "./pages/Live";
import Vod from "./pages/Vod";
import Settings from "./pages/Settings";

type Tab = "live" | "vod" | "sources" | "settings";

const TABS: { key: Tab; label: string; icon: string; hint: string }[] = [
  { key: "live", label: "直播", icon: "◉", hint: "IPTV 频道" },
  { key: "vod", label: "点播", icon: "▤", hint: "影视点播" },
  { key: "sources", label: "源管理", icon: "☰", hint: "订阅与配置" },
  { key: "settings", label: "设置", icon: "⚙", hint: "内核与偏好" },
];

export default function App() {
  // 支持通过地址 hash 直达页面（如 #vod），切换时同步 hash
  const [tab, setTab] = useState<Tab>(() => {
    const h = window.location.hash.replace("#", "") as Tab;
    return TABS.some((t) => t.key === h) ? h : "live";
  });
  useEffect(() => {
    if (window.location.hash !== `#${tab}`) {
      window.history.replaceState(null, "", `#${tab}`);
    }
  }, [tab]);
  const [sources, setSources] = useState<Source[]>([]);
  const [serverOk, setServerOk] = useState<boolean | null>(null);

  const loadSources = () => {
    api
      .get<Source[]>("/api/sources")
      .then(setSources)
      .catch(() => setServerOk(false));
  };

  useEffect(() => {
    api
      .get<{ ok: boolean }>("/api/health")
      .then(() => setServerOk(true))
      .catch(() => setServerOk(false));
    loadSources();
  }, []);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="logo">
          <span className="logo-mark">▶</span>
          <span className="logo-text">
            <b>X-Video</b>
            <small>影视聚合播放器</small>
          </span>
        </div>
        <nav>
          {TABS.map((t) => (
            <div
              key={t.key}
              className={`nav-item${tab === t.key ? " active" : ""}`}
              onClick={() => setTab(t.key)}
              title={t.hint}
            >
              <span className="nav-icon">{t.icon}</span>
              <span className="nav-label">{t.label}</span>
            </div>
          ))}
        </nav>
        <div className="sidebar-foot">
          <span className={serverOk === false ? "dot err" : "dot ok"} />
          <span>{serverOk === false ? "服务未连接" : "本地服务运行中"}</span>
        </div>
      </aside>
      <main className="content">
        {tab === "live" && <Live sources={sources} />}
        {tab === "vod" && <Vod sources={sources} />}
        {tab === "sources" && <Sources sources={sources} onChanged={loadSources} />}
        {tab === "settings" && <Settings />}
      </main>
    </div>
  );
}
