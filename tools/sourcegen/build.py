# -*- coding: utf-8 -*-
"""自建 IPTV / 影视订阅源生成器。

做三件事：
  1. 从多个公开直播清单里收集候选频道，按频道名归一化（CCTV-1 / 湖南卫视 / 翡翠台…）；
  2. 逐个实测（能拉到 playlist、还能拉到第一个分片）只保留真正能播的线路；
  3. 输出本软件可直接订阅的 `iptv.m3u` 与 `vod.json`（TVBox 格式）。

用法：
  python build.py iptv          # 只生成直播源
  python build.py vod           # 只生成影视源
  python build.py all           # 两个都生成
  python build.py all -p        # 生成后提交到 sources 分支（走 jsDelivr 分发）

产物在 dist/ 下。命令行加 -t N 可调整并发（默认 40）。
"""
from __future__ import annotations

import json
import os
import re
import ssl
import sys
import time
import hashlib
import threading
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist"
CACHE = ROOT / ".cache"

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
)

CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE


def log(*a):
    print(*a, flush=True)


def fetch(url: str, timeout: float = 12, limit: int = 4_000_000, headers: dict | None = None):
    h = {"User-Agent": UA, "Accept": "*/*"}
    if headers:
        h.update(headers)
    req = urllib.request.Request(url, headers=h)
    with urllib.request.urlopen(req, timeout=timeout, context=CTX) as f:
        raw = f.read(limit)
        return f.status, f.headers.get("Content-Type", "") or "", raw


def fetch_cached(url: str, timeout: float = 12) -> bytes:
    CACHE.mkdir(parents=True, exist_ok=True)
    key = hashlib.sha1(url.encode()).hexdigest()[:16]
    p = CACHE / f"{key}.bin"
    if p.exists() and time.time() - p.stat().st_mtime < 3600:
        return p.read_bytes()
    _, _, raw = fetch(url, timeout=timeout)
    p.write_bytes(raw)
    return raw


# ---------------------------------------------------------------- 直播清单解析

def parse_m3u(text: str):
    out, pending = [], None
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("#EXTINF"):
            head, _, title = line.partition(",")
            group = re.search(r'group-title="([^"]*)"', head)
            logo = re.search(r'tvg-logo="([^"]*)"', head)
            tvg = re.search(r'tvg-name="([^"]*)"', head)
            name = (title or "").strip() or (tvg.group(1).strip() if tvg else "")
            pending = {
                "name": name,
                "group": (group.group(1).strip() if group else ""),
                "logo": (logo.group(1).strip() if logo else ""),
            }
            continue
        if line.startswith("#"):
            continue
        if pending and "://" in line:
            pending["url"] = line
            out.append(pending)
            pending = None
    return out


def parse_txt(text: str):
    out, group = [], ""
    for raw in text.splitlines():
        line = raw.strip().lstrip("\ufeff")
        if not line or line.startswith("#"):
            continue
        # 两种分组标记都见得到：`xxx,#genre#` 与 `xxx,#group#`
        for marker in (",#genre#", ",#group#"):
            if line.endswith(marker):
                group = line[: -len(marker)].strip()
                line = ""
                break
        if not line:
            continue
        if "," not in line:
            continue
        name, urls = line.split(",", 1)
        for u in urls.split("#"):
            u = u.strip()
            if "://" in u:
                out.append({"name": name.strip(), "url": u, "group": group, "logo": ""})
    return out


def parse_any(text: str):
    return parse_m3u(text) if "#EXTINF" in text else parse_txt(text)


# ---------------------------------------------------------------- 流探测

def _resolve(base: str, rel: str) -> str:
    return urllib.parse.urljoin(base, rel)


def _is_playlist(body: bytes, ctype: str) -> bool:
    head = body[:512].lstrip()
    return head.startswith(b"#EXTM3U") or "mpegurl" in ctype.lower()


def _playlist_ok(url: str, body: bytes, depth: int = 0) -> bool:
    """m3u8 里至少要能拿到一个分片（最多往下钻两层子清单）。"""
    text = body.decode("utf-8", "replace")
    if "#EXT-X-STREAM-INF" in text or (depth == 0 and "#EXTINF" not in text):
        for line in text.splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                sub = _resolve(url, line)
                if depth >= 2:
                    return True
                try:
                    st, _, rb = fetch(sub, timeout=7, limit=65536)
                    if st == 200 and rb:
                        return _playlist_ok(sub, rb, depth + 1)
                except Exception:
                    return False
        return False
    seg = ""
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            seg = line
            break
    if not seg:
        return False
    seg_url = _resolve(url, seg)
    try:
        req = urllib.request.Request(seg_url, headers={"User-Agent": UA, "Range": "bytes=0-4095"})
        with urllib.request.urlopen(req, timeout=7, context=CTX) as f:
            chunk = f.read(4096)
            return f.status in (200, 206) and len(chunk) > 0
    except Exception:
        return False


_PCACHE_PATH = CACHE / "probe.json"
_pcache: dict[str, list] | None = None
_pcache_dirty = False
_pcache_lock = threading.Lock()
PCACHE_TTL = 6 * 3600


def _pcache_load():
    global _pcache
    if _pcache is None:
        try:
            _pcache = json.loads(_PCACHE_PATH.read_text(encoding="utf-8"))
        except Exception:
            _pcache = {}
    return _pcache


def _pcache_flush():
    global _pcache_dirty
    with _pcache_lock:
        if not _pcache_dirty or _pcache is None:
            return
        CACHE.mkdir(parents=True, exist_ok=True)
        _PCACHE_PATH.write_text(json.dumps(_pcache, ensure_ascii=False), encoding="utf-8")
        _pcache_dirty = False


def probe_cached(url: str) -> tuple[bool, str, int]:
    global _pcache_dirty
    p = _pcache_load()
    hit = p.get(url)
    if hit and time.time() - hit[3] < PCACHE_TTL:
        return bool(hit[0]), hit[1], int(hit[2])
    ok, info, ms = probe(url)
    with _pcache_lock:
        p[url] = [ok, info, ms, time.time()]
        _pcache_dirty = True
    return ok, info, ms


def probe(url: str) -> tuple[bool, str, int]:
    """返回 (可播, 说明, 耗时ms)。"""
    t0 = time.time()
    try:
        st, ctype, body = fetch(url, timeout=8, limit=131072)
    except Exception as e:
        return False, type(e).__name__, int((time.time() - t0) * 1000)
    ms = int((time.time() - t0) * 1000)
    if st != 200:
        return False, f"HTTP {st}", ms
    if _is_playlist(body, ctype):
        ok = _playlist_ok(url, body)
        return ok, "hls" if ok else "hls-dead", int((time.time() - t0) * 1000)
    if body[:3] == b"FLV":
        return True, "flv", ms
    if body[:1] == b"\x47":  # MPEG-TS sync byte
        return True, "ts", ms
    if b"ftyp" in body[:32]:
        return True, "mp4", ms
    if ctype.startswith(("video/", "audio/")) or "octet-stream" in ctype:
        return True, ctype.split(";")[0], ms
    return False, f"ct={ctype or '-'}", ms


# ---------------------------------------------------------------- 频道名归一化

GROUP_ORDER = ["央视频道", "卫视", "港澳台", "地方频道", "少儿", "其他"]

JUNK = re.compile(r"更新时间|支持作者|广告|測試|测试|DEMO|ABOUT|预告|花絮|失效|请勿|捐赠", re.I)
BAD_URL = re.compile(r"(?i)\.(mp4|mkv|avi|flv|mp3|aac|jpg|jpeg|png|gif|webp|html?|php\?.*=about)(\?|$)")
# 直播不该出现的点播 CDN
BAD_HOST = re.compile(
    r"(?i)(kwimgs\.com|bdstatic\.com|bilivideo\.com|douyinvod\.com|ixigua\.com|gitee\.com/.*asset|github\.com/.*/raw)"
)

CCTV_OK = set(range(1, 18)) | {5}

# 港澳台 / 少儿 的识别规则（用正则而不是子串，避免「澳门风云」「TVBRICS」被误判）
HKMO_TW = [
    r"凤凰(中文|资讯|香港|卫视)",
    r"翡翠台|明珠台|无线新闻|无线财经",
    r"TVB(?!S|RICS)",
    r"VIUTV|HOY",
    r"澳视|澳门莲花|澳门卫视",
    r"中天|东森|三立|民视|台视|中视|华视|纬来|八大|龙华|靖天",
]
KIDS = ("卡酷", "金鹰卡通", "嘉佳卡通", "优漫卡通", "炫动卡通", "动漫秀场", "新动漫", "哈哈炫动")

LOCAL_TAIL = re.compile(r"(台|频道|综合|新闻|都市|影视|生活|公共|经济|体育|纪实|科教|文艺|TV)$")


def classify_name(raw: str):
    """把源里的各种写法归一到统一频道名；返回 (显示名, 归属组) 或 (None, None)。"""
    if not raw:
        return None, None
    if JUNK.search(raw):
        return None, None
    n = re.sub(r"[\s\u3000]+", "", raw)
    n = re.sub(r"[\(（\[【][^\)）\]】]*[\)）\]】]", "", n).strip("-·_ ")

    # 央视
    m = re.match(r"(?i)^(CCTV|央视|中央)[-_]?(\d{1,2})(\+)?", n)
    if m:
        num = int(m.group(2))
        if num not in CCTV_OK:
            return None, None
        plus = "+" if m.group(3) else ""
        return f"CCTV-{num}{plus}", "央视频道"
    if re.match(r"(?i)^CGTN", n):
        tail = re.sub(r"(?i)^CGTN[-_]?", "", n)
        tail = re.sub(r"(?i)(HD|高清|SD)$", "", tail)
        return ("CGTN" + (f"-{tail}" if tail else "")), "央视频道"

    # 卫视：取「X卫视」前缀，丢掉后面的修饰
    m = re.search(r"([\u4e00-\u9fa5]{2,5}卫视)", n)
    if m:
        return m.group(1), "卫视"

    if len(n) <= 8:
        for pat in HKMO_TW:
            if re.search(pat, n, re.I):
                return re.sub(r"(?i)(HD|高清|超清|1080P|720P)$", "", n), "港澳台"
        up = n.upper()
        for k in KIDS:
            if k.upper() in up:
                return re.sub(r"(?i)(HD|高清|超清|1080P|720P)$", "", n), "少儿"

    # 其余台：形如「XX新闻综合」「广州综合」「太谷新闻综合」
    if 2 <= len(n) <= 8 and LOCAL_TAIL.search(n) and not JUNK.search(n):
        return n, "地方频道"
    return None, None


def url_ok(url: str) -> bool:
    low = url.lower()
    if not low.startswith(("http://", "https://")):
        return False  # rtp:// igmp:// udp:// 只在运营商内网可用
    if BAD_URL.search(low.split("?")[0]):
        return False
    if BAD_HOST.search(low):
        return False
    return True


# ---------------------------------------------------------------- IPTV 生成

LIVE_LISTS = [
    # 标清/高清混合清单，频道覆盖全，线路多
    ("fanmingming-index", "https://cdn.jsdelivr.net/gh/fanmingming/live@main/tv/m3u/index.m3u"),
    ("fanmingming-itv", "https://cdn.jsdelivr.net/gh/fanmingming/live@main/tv/m3u/itv.m3u"),
    ("vbskycn", "https://cdn.jsdelivr.net/gh/vbskycn/iptv@master/tv/iptv4.txt"),
    ("YueChan", "https://cdn.jsdelivr.net/gh/YueChan/Live@main/IPTV.m3u"),
    ("ssili-zb", "https://cdn.jsdelivr.net/gh/ssili126/tv@main/zbchannel.txt"),
    ("kimwang-lite", "https://cdn.jsdelivr.net/gh/kimwang1978/collect-tv-txt@main/bbxx365_lite.txt"),
    ("iptv-org", "https://iptv-org.github.io/iptv/countries/cn.m3u"),
]


def build_iptv(max_per_channel: int = 4, workers: int = 40, extra_cap: int = 240):
    cands: list[dict] = []
    stats = {}
    for label, url in LIVE_LISTS:
        try:
            text = fetch_cached(url, timeout=25).decode("utf-8", "replace")
            got = parse_any(text)
        except Exception as e:
            log(f"  {label:18s} 拉取失败 {type(e).__name__}")
            continue
        kept = 0
        for c in got:
            c["from"] = label
            cands.append(c)
            kept += 1
        stats[label] = (len(got), kept)
        log(f"  {label:18s} 候选 {len(got)}")

    # 归一化 + 过滤 + 去重（同频道同地址只留一条）
    seen: set[tuple[str, str]] = set()
    uniq: list[dict] = []
    extra: list[dict] = []
    for c in cands:
        if not url_ok(c.get("url", "")):
            continue
        disp, group = classify_name(c["name"])
        if not disp:
            continue
        key = (disp, c["url"].strip())
        if key in seen:
            continue
        seen.add(key)
        item = dict(c, name=disp, group2=group)
        (extra if group == "地方频道" else uniq).append(item)

    # 地方台只从「小而精」的清单里取，避免上万条无效探测
    extra = [c for c in extra if c["from"] in ("vbskycn", "fanmingming-index", "fanmingming-itv", "YueChan")]
    extra.sort(key=lambda c: c["from"])
    extra = extra[:extra_cap]
    todo = uniq + extra
    log(f"  归一化后待测 {len(todo)} 条（其中地方台 {len(extra)}），开始实测…")

    results: list[dict] = []
    done = 0
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = [ex.submit(probe_cached, c["url"]) for c in todo]
        for c, fut in zip(todo, futs):
            ok, info, ms = fut.result()
            done += 1
            if done % 200 == 0:
                log(f"    {done}/{len(todo)}，可播 {len(results)}")
            if ok:
                results.append(dict(c, info=info, ms=ms))
    _pcache_flush()
    log(f"  实测完成：可播线路 {len(results)}/{len(todo)}")

    by_name: dict[str, list[dict]] = {}
    for r in sorted(results, key=lambda x: x["ms"]):
        by_name.setdefault(r["name"], []).append(r)

    groups: dict[str, list[dict]] = {g: [] for g in GROUP_ORDER}
    for name, items in by_name.items():
        group = items[0]["group2"]
        groups.setdefault(group, []).append(
            {
                "name": name,
                "sources": items[:max_per_channel],
                "logo": next((i["logo"] for i in items if i.get("logo")), ""),
            }
        )

    # 地方台太多没意义，按可用线路数留前 40
    if len(groups["地方频道"]) > 40:
        groups["地方频道"].sort(key=lambda x: -len(x["sources"]))
        groups["地方频道"] = groups["地方频道"][:40]

    lines = [
        "#EXTM3U",
        "#PLAYLIST: X-Video 自建直播源 · 全线路实测可用 · 同名频道多源自动切换",
        f'#UPDATED: {time.strftime("%Y-%m-%d %H:%M:%S")}',
    ]
    total = 0
    summary = {}
    for g in GROUP_ORDER:
        items = groups.get(g) or []
        if not items:
            continue
        lines.append("")
        # 央视按台号排，其它按名字排
        def sort_key(x):
            m = re.match(r"CCTV-(\d+)", x["name"])
            return (0, int(m.group(1))) if m else (1, x["name"])

        items.sort(key=sort_key)
        summary[g] = len(items)
        for it in items:
            for idx, src in enumerate(it["sources"]):
                attr = [f'group-title="{g}"', f'tvg-name="{it["name"]}"']
                if it["logo"]:
                    attr.append(f'tvg-logo="{it["logo"]}"')
                suffix = "" if idx == 0 else f" {idx + 1}"
                lines.append(f"#EXTINF:-1 {' '.join(attr)},{it['name']}{suffix}")
                lines.append(src["url"])
                total += 1

    DIST.mkdir(parents=True, exist_ok=True)
    out = DIST / "iptv.m3u"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    log(f"  → {out}  {sum(summary.values())} 个频道 / {total} 条线路")
    for g in GROUP_ORDER:
        if summary.get(g):
            log(f"     {g:6s} {summary[g]}")
    return {"channels": sum(summary.values()), "lines": total}


# ---------------------------------------------------------------- 影视生成

CONFIGS = [
    ("gao-js", "https://cdn.jsdelivr.net/gh/gaotianliuyun/gao@master/js.json"),
    ("gao-0825", "https://cdn.jsdelivr.net/gh/gaotianliuyun/gao@master/0825.json"),
    ("gao-0821", "https://cdn.jsdelivr.net/gh/gaotianliuyun/gao@master/0821.json"),
    ("hgyx", "https://api.hgyx.vip/hgyx.json"),
    ("weixine", "https://weixine.net/ysc.json"),
    ("pandown", "http://pandown.pro/tvbox/tvbox.json"),
    ("xhztv", "http://xhztv.top/tvbox.txt"),
    ("tvkj", "http://tvkj.top/DC.txt"),
]

# 公开采集站种子（TVBox 生态里常见的 type=1 CMS 接口），会逐个实测淘汰
SEED_APIS = [
    ("量子资源", "https://cj.lziapi.com/api.php/provide/vod/"),
    ("非凡资源", "http://cj.ffzyapi.com/api.php/provide/vod/"),
    ("暴风资源", "https://bfzyapi.com/api.php/provide/vod/"),
    ("急速资源", "https://jszyapi.com/api.php/provide/vod/"),
    ("天涯资源", "https://tyyszy.com/api.php/provide/vod/"),
    ("无尽资源", "https://api.wujinapi.me/api.php/provide/vod/"),
    ("索尼资源", "https://suoniapi.com/api.php/provide/vod/"),
    ("ikun资源", "https://ikunzyapi.com/api.php/provide/vod/"),
    ("魔都资源", "https://www.mdzyapi.com/api.php/provide/vod/"),
    ("卧龙资源", "https://collect.wolongzyw.com/api.php/provide/vod/"),
    ("火狐资源", "https://hhzyapi.com/api.php/provide/vod/"),
    ("如意资源", "https://cj.rycjapi.com/api.php/provide/vod/"),
    ("百度云资源", "https://api.apibdzy.com/api.php/provide/vod/"),
    ("快看资源", "https://kuaikan-api.com/api.php/provide/vod/"),
    ("最大资源", "https://api.zuidapi.com/api.php/provide/vod/"),
    ("红牛资源", "https://www.hongniuzy2.com/api.php/provide/vod/"),
    ("光速资源", "https://api.guangsuapi.com/api.php/provide/vod/"),
    ("新浪资源", "https://api.xinlangapi.com/api.php/provide/vod/"),
    ("牛牛资源", "https://api.niuniuzy.me/api.php/provide/vod/"),
    ("天空资源", "https://api.tiankongapi.com/api.php/provide/vod/"),
    ("金鹰资源", "https://jyzyapi.com/api.php/provide/vod/"),
    ("辣椒资源", "https://apilj.com/api.php/provide/vod/"),
    ("极速资源", "https://jszyapi.com/api.php/provide/vod/"),
    ("快车资源", "https://caiji.kczyapi.com/api.php/provide/vod/"),
]


def _api_call(api: str, params: dict):
    sep = "&" if "?" in api else "?"
    q = urllib.parse.urlencode(params)
    try:
        _, _, raw = fetch(f"{api}{sep}{q}", timeout=10, limit=2_000_000)
        return json.loads(raw.decode("utf-8", "replace"))
    except Exception:
        return None


def test_cms(name: str, api: str, keyword: str) -> dict | None:
    """搜索 → 详情 → 逐条线路实测首集，任意一条能播就算可用。"""
    t0 = time.time()
    d = _api_call(api, {"ac": "videolist", "wd": keyword, "pg": 1})
    if not d or not isinstance(d.get("list"), list) or not d["list"]:
        return None
    first = d["list"][0]
    vid = first.get("vod_id")
    title = first.get("vod_name") or ""
    detail = _api_call(api, {"ac": "videolist", "ids": vid}) or {}
    dlist = detail.get("list") or []
    play = ""
    if dlist:
        play = str(dlist[0].get("vod_play_url") or "")
        title = dlist[0].get("vod_name") or title

    # 每条线路取首集，优先测 m3u8（分享页跳过），最多试 4 条
    eps: list[str] = []
    for line in play.split("$$$"):
        for part in line.split("#"):
            if "$" not in part:
                continue
            u = part.split("$", 1)[1].strip()
            if not u.startswith("http"):
                continue
            low = u.lower().split("?")[0]
            if low.endswith((".m3u8", ".mp4", ".flv", ".ts")):
                eps.append(u)
                break
    if not eps:
        return {"name": name, "api": api, "hit_title": title[:24], "ep": "",
                "playable": False, "ms": int((time.time() - t0) * 1000)}

    playable = False
    ok_ep = ""
    for u in eps[:4]:
        if probe_cached(u)[0]:
            playable, ok_ep = True, u
            break
    return {
        "name": name,
        "api": api,
        "hit_title": title[:24],
        "ep": (ok_ep or eps[0])[:100],
        "playable": playable,
        "ms": int((time.time() - t0) * 1000),
    }


def collect_cms() -> list[dict]:
    seen: set[str] = set()
    out: list[dict] = []

    def host_key(u: str) -> str:
        return urllib.parse.urlparse(u).netloc.lower()

    for name, api in SEED_APIS:
        k = host_key(api)
        if k and k not in seen:
            seen.add(k)
            out.append({"name": name, "api": api})

    for label, url in CONFIGS:
        try:
            cfg = json.loads(fetch_cached(url, timeout=25).decode("utf-8", "replace"))
        except Exception as e:
            log(f"  {label:10s} 配置拉取失败 {type(e).__name__}")
            continue
        sites = cfg.get("sites") or []
        kept = 0
        for s in sites:
            api = (s.get("api") or "").strip()
            if s.get("type") != 1 or not api.startswith("http") or s.get("searchable") == 0:
                continue
            k = host_key(api)
            if not k or k in seen:
                continue
            seen.add(k)
            out.append({"name": s.get("name") or k, "api": api})
            kept += 1
        log(f"  {label:10s} 站点 {len(sites)}，新增原生 CMS {kept}")
    return out


def build_vod(workers: int = 12, keep_max: int = 24):
    keyword = os.environ.get("XV_PROBE_KEYWORD", "庆余年")
    cands = collect_cms()
    log(f"  原生 CMS 候选 {len(cands)} 个，实测（搜索 → 详情 → 首集可播）…")
    good: list[dict] = []
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = [ex.submit(test_cms, c["name"], c["api"], keyword) for c in cands]
        for c, fut in zip(cands, futs):
            try:
                r = fut.result()
            except Exception:
                r = None
            if not r:
                continue
            log(f"    {'✅' if r['playable'] else '○'} {r['name'][:14]:14s} 《{r['hit_title']}》 {r['ms']:5d}ms {r['api'][:50]}")
            if r["playable"]:
                good.append(r)

    good.sort(key=lambda x: x["ms"])
    good = good[:keep_max]
    log(f"  可播放站点 {len(good)} 个")

    sites = []
    for i, g in enumerate(good):
        sites.append(
            {
                "key": f"xv{i}_{re.sub(r'[^a-z0-9]', '', g['name'].lower()) or 'cms'}",
                "name": g["name"],
                "type": 1,
                "api": g["api"],
                "searchable": 1,
                "quickSearch": 1,
                "filter": 1,
            }
        )

    cfg = {
        "name": "X-Video 自建影视源",
        "version": time.strftime("%Y%m%d%H%M"),
        "sites": sites,
        "lives": [{"name": "自建直播", "url": live_url(), "ua": UA}],
        "parses": [],
    }
    DIST.mkdir(parents=True, exist_ok=True)
    out = DIST / "vod.json"
    out.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
    log(f"  → {out}  {len(sites)} 站点")
    return {"sites": len(sites)}


# ---------------------------------------------------------------- 发布

PUBLISH_BRANCH = "sources"
PUBLISH_BASE = "https://cdn.jsdelivr.net/gh/xfm0797/x-video@{branch}/{path}"


def live_url() -> str:
    return PUBLISH_BASE.format(branch=PUBLISH_BRANCH, path="iptv.m3u")


def vod_url() -> str:
    return PUBLISH_BASE.format(branch=PUBLISH_BRANCH, path="vod.json")


def publish(repo: Path):
    """把 dist/ 下的产物提交到 sources 分支（不动 main）。"""
    import subprocess

    def run(*args, cwd=repo, check=True):
        r = subprocess.run(args, cwd=str(cwd), capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        if check and r.returncode != 0:
            raise SystemExit(f"命令失败：{' '.join(args)}\n{r.stdout}\n{r.stderr}")
        return r.stdout

    for f in ("iptv.m3u", "vod.json"):
        if not (DIST / f).exists():
            raise SystemExit(f"缺少产物 dist/{f}，先跑 build")

    tmp = repo / ".sources-work"
    run("git", "worktree", "remove", "--force", str(tmp), check=False)
    run("git", "fetch", "origin", PUBLISH_BRANCH, check=False)
    exists = run("git", "ls-remote", "--heads", "origin", PUBLISH_BRANCH).strip() != ""
    if exists:
        run("git", "worktree", "add", str(tmp), PUBLISH_BRANCH)
        run("git", "pull", "--ff-only", "origin", PUBLISH_BRANCH, cwd=tmp, check=False)
    else:
        run("git", "worktree", "add", "--detach", str(tmp))
        run("git", "checkout", "--orphan", PUBLISH_BRANCH, cwd=tmp)
        run("git", "rm", "-rf", "--cached", ".", cwd=tmp, check=False)
    for f in ("iptv.m3u", "vod.json"):
        (tmp / f).write_bytes((DIST / f).read_bytes())
    (tmp / "README.md").write_text(
        "# X-Video 自建订阅源\n\n"
        "本目录由 `tools/sourcegen/build.py` 自动生成，请勿手改。\n\n"
        f"- 直播：`{live_url()}`\n- 影视：`{vod_url()}`\n",
        encoding="utf-8",
    )
    run("git", "add", "-A", cwd=tmp)
    run("git", "-c", "user.name=xv-sourcegen", "-c", "user.email=noreply@local",
        "commit", "-m", f"chore(sources): 更新订阅源 {time.strftime('%Y-%m-%d %H:%M')}",
        cwd=tmp, check=False)
    run("git", "push", "origin", PUBLISH_BRANCH, cwd=tmp)
    run("git", "worktree", "remove", "--force", str(tmp), check=False)
    log(f"  已发布：{live_url()}")
    log(f"          {vod_url()}")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    flags = sys.argv[1:]
    what = args[0] if args else "all"

    workers = 40
    if "-t" in flags:
        try:
            workers = int(flags[flags.index("-t") + 1])
        except Exception:
            pass

    if what in ("iptv", "all"):
        log("== 生成直播源 ==")
        build_iptv(workers=workers)
    if what in ("vod", "all"):
        log("== 生成影视源 ==")
        build_vod()
    if "-p" in flags or "--publish" in flags:
        log("== 发布到 sources 分支 ==")
        publish(ROOT.parent.parent)


if __name__ == "__main__":
    main()
