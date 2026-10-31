export default function Settings() {
  return (
    <div className="page">
      <h2>设置</h2>
      <div className="card">
        <h3>关于</h3>
        <p className="hint">
          X-Video v0.1.1 · Tauri 2 + React + Rust
        </p>
        <p className="hint">
          本地服务地址：http://127.0.0.1:21600（API 与流媒体代理）
        </p>
      </div>
      <div className="card">
        <h3>路线图</h3>
        <ul className="roadmap">
          <li>✅ 源管理（M3U / TXT / TVBox 配置）</li>
          <li>✅ IPTV 直播播放（含 m3u8 代理重写）</li>
          <li>✅ TVBox type=1 JSON CMS 点播（分类 / 列表 / 详情 / 播放 / 搜索）</li>
          <li>🚧 libmpv 播放内核（FLV/RMVB/DASH 及更多格式）</li>
          <li>🚧 JS Spider 支持（QuickJS 沙箱）</li>
          <li>🚧 聚合搜索、收藏与播放历史、EPG 节目单</li>
          <li>🚧 自动更新</li>
        </ul>
      </div>
    </div>
  );
}
