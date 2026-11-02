import { useEffect, useState } from "react";
import { api, type Status } from "../lib/api";
import { fmtRelative } from "../lib/format";
import { Icon } from "../components/Icon";

type Props = {
  status: Status | null;
  onChanged: () => void;
  toast: (msg: string) => void;
};

type Entry = { name: string; path: string };

export function Files({ status, onChanged, toast }: Props) {
  const [roots, setRoots] = useState<string[]>([]);
  const [browsing, setBrowsing] = useState(false);
  const [cwd, setCwd] = useState("");
  const [parent, setParent] = useState("");
  const [dirs, setDirs] = useState<Entry[]>([]);
  const [drives, setDrives] = useState<Entry[]>([]);
  const [busy, setBusy] = useState(false);

  const loadRoots = () => api.roots().then((r) => setRoots(r.items)).catch(() => {});

  useEffect(() => {
    loadRoots();
    api.drives().then((r) => setDrives(r.items)).catch(() => {});
  }, []);

  const browse = async (path: string) => {
    try {
      const r = await api.ls(path);
      setCwd(r.path);
      setParent(r.parent);
      setDirs(r.items);
    } catch (e) {
      toast(String(e));
    }
  };

  const openBrowser = async () => {
    setBrowsing(true);
    if (drives.length > 0) await browse(cwd || drives[0].path);
  };

  const addAndScan = async () => {
    if (!cwd) return;
    setBusy(true);
    try {
      await api.addRoot(cwd);
      await api.scan(cwd);
      await loadRoots();
      onChanged();
      toast(`已开始扫描：${cwd}`);
      setBrowsing(false);
    } catch (e) {
      toast(String(e));
    } finally {
      setBusy(false);
    }
  };

  const rescan = async (path: string) => {
    try {
      await api.scan(path);
      toast(`已开始扫描：${path}`);
    } catch (e) {
      toast(String(e));
    }
  };

  const remove = async (path: string) => {
    try {
      const r = await api.removeRoot(path);
      await loadRoots();
      onChanged();
      toast(`已移除，清理 ${r.removed} 条索引`);
    } catch (e) {
      toast(String(e));
    }
  };

  return (
    <>
      <div className="panel">
        <h3>媒体目录</h3>
        <p className="hint">
          添加存放电影、剧集、短剧或音乐的文件夹。扫描只读取文件名与大小，
          编码与分辨率会在播放或生成封面时按需补齐。
        </p>

        {roots.length === 0 && (
          <div style={{ color: "var(--dim-2)", fontSize: 12.5, padding: "10px 0 16px" }}>
            还没有添加目录。
          </div>
        )}

        {roots.map((r) => (
          <div className="root-item" key={r}>
            <Icon name="folder" size={16} />
            <span className="path" title={r}>
              {r}
            </span>
            <button className="btn" onClick={() => rescan(r)}>
              扫描
            </button>
            <button className="btn" onClick={() => remove(r)}>
              移除
            </button>
          </div>
        ))}

        <div style={{ display: "flex", gap: 10, marginTop: 14, alignItems: "center" }}>
          <button className="btn primary" onClick={openBrowser}>
            添加文件夹
          </button>
          {status?.scanning && (
            <span style={{ fontSize: 12.5, color: "var(--teal)" }}>正在扫描…</span>
          )}
          {!status?.scanning && status?.last_scan && (
            <span style={{ fontSize: 12.5, color: "var(--dim-2)" }}>
              上次扫描：新增 {status.last_scan.added} · 更新 {status.last_scan.updated} ·
              跳过 {status.last_scan.skipped} · 用时{" "}
              {(status.last_scan.elapsed_ms / 1000).toFixed(1)}s
            </span>
          )}
        </div>
      </div>

      <div className="panel">
        <h3>目录状态</h3>
        <div className="kv">
          <span>索引条目</span>
          <b>
            {status
              ? `${status.counts.movie} 电影 / ${status.counts.tv} 剧集 / ${status.counts.short} 短剧 / ${status.counts.music} 专辑`
              : "—"}
          </b>
        </div>
        <div className="kv">
          <span>数据目录</span>
          <b>{status?.data_dir || "—"}</b>
        </div>
        <div className="kv">
          <span>运行时长</span>
          <b>{status ? fmtRelative(Date.now() / 1000 - status.uptime) : "—"}</b>
        </div>
      </div>

      {browsing && (
        <div className="modal-mask" onClick={() => setBrowsing(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3 style={{ margin: "0 0 4px", fontSize: 14 }}>选择媒体文件夹</h3>
            <p className="hint" style={{ margin: "0 0 14px" }}>
              进入目标目录后点「添加并扫描」，其子目录会被递归收录。
            </p>

            <div className="fs-path">{cwd || "…"}</div>

            <div style={{ display: "flex", gap: 8, marginBottom: 10, flexWrap: "wrap" }}>
              {parent && (
                <button className="btn" onClick={() => browse(parent)}>
                  上一级
                </button>
              )}
              {drives.map((d) => (
                <button key={d.path} className="btn" onClick={() => browse(d.path)}>
                  {d.name}
                </button>
              ))}
            </div>

            <div className="fs-list">
              {dirs.length === 0 && (
                <div style={{ color: "var(--dim-2)", fontSize: 12.5, padding: 10 }}>
                  这个目录下没有子文件夹，可以直接添加。
                </div>
              )}
              {dirs.map((d) => (
                <button key={d.path} className="fs-row" onClick={() => browse(d.path)}>
                  <Icon name="folder" size={15} />
                  <span>{d.name}</span>
                  <Icon name="chevron" size={14} />
                </button>
              ))}
            </div>

            <div style={{ display: "flex", gap: 10, marginTop: 16, justifyContent: "flex-end" }}>
              <button className="btn" onClick={() => setBrowsing(false)}>
                取消
              </button>
              <button className="btn primary" onClick={addAndScan} disabled={busy || !cwd}>
                {busy ? "处理中…" : "添加并扫描"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
