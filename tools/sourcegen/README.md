# 自建订阅源（sourcegen）

给 X-Video 生成**自己维护**的 IPTV 直播源与影视源，不再依赖第三方随时失效的配置。

## 它怎么工作

```
公开清单（多个） → 归一化频道名 → 逐条实测（playlist + 首分片） → 只留活线路 → iptv.m3u / vod.json
```

- **直播**：抓 `fanmingming/live`、`vbskycn/iptv`、`YueChan/Live`、`ssili126/tv`、
  `kimwang1978/collect-tv-txt` 等公开清单，按 `CCTV-1` / `湖南卫视` / `翡翠台` 归一化，
  过滤点播 mp4、组播 `rtp://`、垃圾条目后逐个探测：能拉到 m3u8、还能拉到第一个分片才算活。
  同名频道保留最多 4 条可用线路，应用里点一次会自动按顺序切换。
- **影视**：收集 TVBox 生态里的 `type=1` 原生 CMS 接口（配置里的 `sites` + 内置种子），
  对每个站点走一遍「搜索 → 详情 → 首集地址实测」，只有首集真能拉流才写进配置。
  需要 Java/JS 内核的 `type=3` 站点不收录（本程序跑不了）。

## 用法

```bash
python tools/sourcegen/build.py iptv        # 只生成直播源
python tools/sourcegen/build.py vod         # 只生成影视源
python tools/sourcegen/build.py all -p      # 两个都生成并发布到 sources 分支
```

产物写在 `tools/sourcegen/dist/`：
`iptv.m3u`（M3U 直播清单）、`vod.json`（TVBox 格式配置，`lives` 指向自建直播源）。

`-p` 会把产物提交到本仓库的 `sources` 干净分支（不动 `main`），
再由 jsDelivr 分发，订阅地址固定为：

```
直播  https://cdn.jsdelivr.net/gh/xfm0797/x-video@sources/iptv.m3u
影视  https://cdn.jsdelivr.net/gh/xfm0797/x-video@sources/vod.json
```

jsDelivr 对分支文件有约 12 小时缓存，更新后稍等即生效。

## 在应用里订阅

设置 → 订阅 → 添加，粘贴上面的地址即可（一个 `iptv` 类型、一个 `tvbox` 类型）。
应用每 4 秒刷新状态、订阅过期 6 小时后自动重拉，无需手动维护。

## 可调参数

| 参数 | 说明 |
| --- | --- |
| `-t N` | 探测并发（默认 40），网络差可调小 |
| `XV_PROBE_KEYWORD` | 影视站验证用的搜索词（默认「庆余年」） |
| `--publish` / `-p` | 生成后提交到 `sources` 分支 |

探测缓存放在 `.cache/`（清单 1 小时内不重复下载），已加入 `.gitignore`。
