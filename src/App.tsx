import { useEffect, useState } from "react";
import { api, type Source } from "./lib/api";
import Sources from "./pages/Sources";
import Live from "./pages/Live";
import Vod from "./pages/Vod";
import Settings from "./pages/Settings";

type Tab = "live" | "vod" | "sources" | "settings";

const TABS: { key: Tab; label: string; icon: string }[] = [
  { key: "live", label: "直播", icon: "▣" },
  { key: "vod", label: "点播", icon: "▤" },
  { key: "sources", label: "源管理", icon: "☰" },
  { key: "settings", label: "设置", icon: "⚙" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("live");
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
          <span>X-Video</span>
        </div>
        <nav>
          {TABS.map((t) => (
            <div
              key={t.key}
              className={`nav-item${tab === t.key ? " active" : ""}`}
              onClick={() => setTab(t.key)}
            >
              <span className="nav-icon">{t.icon}</span>
              {t.label}
            </div>
          ))}
        </nav>
        <div className="sidebar-foot">
          <span className={serverOk === false ? "dot err" : "dot ok"} />
          {serverOk === false ? "服务未连接" : "本地服务运行中"}
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
