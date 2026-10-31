// 仅用于本地视觉验证的假 API（不参与打包）
import http from "node:http";

const poster = (title, h1, h2) => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="450">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="${h1}"/><stop offset="1" stop-color="${h2}"/></linearGradient></defs>
  <rect width="300" height="450" fill="url(#g)"/>
  <circle cx="230" cy="90" r="90" fill="rgba(255,255,255,0.12)"/>
  <text x="24" y="400" font-size="30" font-family="sans-serif" fill="rgba(255,255,255,0.92)">${title}</text>
</svg>`;
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
};

const NAMES = [
  "星际穿越", "长安十二时辰", "沙丘 2", "漫长的季节", "流浪地球 3",
  "狂飙", "三体", "奥本海默", "繁花", "看不见的客人",
  "心灵奇旅", "瞬息全宇宙", "隐入尘烟", "宇宙探索编辑部", "满江红",
  "孤注一掷", "消失的她", "八佰", "刺杀小说家", "第二十条",
];
const HUES = ["#3b2f8f,#7c5cff", "#0f4c5c,#22d3ee", "#5c2b2b,#ff8a5c", "#233b2f,#34d399", "#2b2350,#a78bfa"];

const items = NAMES.map((n, i) => ({
  vod_id: String(1000 + i),
  vod_name: n,
  vod_pic: poster(n, ...HUES[i % HUES.length].split(",")),
  vod_remarks: i % 3 === 0 ? "更新至 24 集" : i % 3 === 1 ? "HD 4K" : "全 40 集",
}));

const json = (res, data) => {
  res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
  res.end(JSON.stringify(data));
};

http
  .createServer((req, res) => {
    const u = new URL(req.url, "http://x");
    const p = u.pathname;
    if (p === "/api/health") return json(res, { ok: true });
    if (p === "/api/sources")
      return json(res, [
        {
          id: "s1", name: "演示 TVBox 配置", type: "tvbox",
          url: "https://example.com/demo.json", enabled: true, added_at: 0,
          channel_count: 226, site_count: 18, last_error: null,
        },
        {
          id: "s2", name: "IPTV 中文频道", type: "m3u",
          url: "https://iptv-org.github.io/iptv/languages/zho.m3u", enabled: true,
          added_at: 0, channel_count: 226, site_count: 0, last_error: null,
        },
      ]);
    if (p.endsWith("/sites"))
      return json(res, {
        sites: [
          { key: "demo", name: "演示 CMS 站点", site_type: 1, api: "https://example.com/api.php/provide/vod/", searchable: 1 },
          { key: "js1", name: "JS Spider 站点", site_type: 3, api: "csp_demo", searchable: 1 },
        ],
      });
    if (p.endsWith("/home"))
      return json(res, [
        { type_id: "1", type_name: "电影" },
        { type_id: "2", type_name: "电视剧" },
        { type_id: "3", type_name: "综艺" },
        { type_id: "4", type_name: "动漫" },
        { type_id: "5", type_name: "纪录片" },
      ]);
    if (p.endsWith("/list"))
      return json(res, { list: items, pagecount: 6, total: 120, pg: 1 });
    if (p.endsWith("/detail"))
      return json(res, {
        vod_id: "1000", vod_name: "星际穿越", vod_pic: items[0].vod_pic,
        vod_year: "2014", vod_area: "美国", vod_type: "科幻 / 冒险",
        vod_actor: "马修·麦康纳 / 安妮·海瑟薇", vod_director: "克里斯托弗·诺兰",
        vod_content:
          "近未来地球环境恶化，一群探险家利用他们对虫洞的发现，超越人类对太空旅行的极限，在星际间寻找新家园。",
        play_sources: [
          { from: "量子", episodes: [["第1集", "http://a/1"], ["第2集", "http://a/2"], ["第3集", "http://a/3"]] },
          { from: "暴风", episodes: [["HD中字", "http://b/1"]] },
        ],
      });
    if (p.startsWith("/api/live/"))
      return json(res, [
        {
          name: "央视",
          channels: Array.from({ length: 12 }, (_, i) => ({
            name: `CCTV-${i + 1}`, url: `http://x/${i}.m3u8`,
          })),
        },
        {
          name: "卫视",
          channels: Array.from({ length: 10 }, (_, i) => ({
            name: ["湖南卫视", "浙江卫视", "江苏卫视", "东方卫视", "北京卫视"][i % 5] + (i > 4 ? " 高清" : ""),
            url: `http://x/s${i}.m3u8`,
          })),
        },
      ]);
    res.writeHead(404).end("{}");
  })
  .listen(21600, "127.0.0.1", () => console.log("mock api on 21600"));
