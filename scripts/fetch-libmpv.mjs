// 打包前拉取 libmpv 内核（Windows x64）。
// 来源：shinchiro / zhongfly 的 mpv-winbuild 自动构建（mpv-dev 包里的 libmpv-2.dll）。
// 解压链：系统 tar(bsdtar) → 7-Zip → 现下载 7zr.exe，任一成功即可。
// 可用 XVIDEO_MPV_ASSET 指定固定版本、XVIDEO_MPV_DLL 指定本地 dll，便于复现构建。
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readdirSync,
  statSync,
  unlinkSync,
} from "node:fs";
import { createReadStream } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "../src-tauri/resources/mpv");
const outFile = join(outDir, "libmpv-2.dll");
const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN || "";

function headers(extra = {}) {
  return {
    "user-agent": "x-video-build",
    accept: "application/vnd.github+json",
    ...(token ? { authorization: `Bearer ${token}` } : {}),
    ...extra,
  };
}

async function downloadTo(url, dest) {
  const res = await fetch(url, { headers: headers({ accept: "application/octet-stream" }) });
  if (!res.ok) throw new Error(`下载失败 ${res.status} ${res.statusText} ← ${url}`);
  await pipeline(Readable.fromWeb(res.body), createWriteStream(dest));
  const size = statSync(dest).size;
  console.log(`[mpv] 已下载 ${(size / 1048576).toFixed(1)} MB → ${dest}`);
  return size;
}

// ---------- 本地 dll 直用 ----------
if (process.env.XVIDEO_MPV_DLL && existsSync(process.env.XVIDEO_MPV_DLL)) {
  mkdirSync(outDir, { recursive: true });
  await pipeline(createReadStream(process.env.XVIDEO_MPV_DLL), createWriteStream(outFile));
  console.log(`[mpv] 使用本地内核 ${process.env.XVIDEO_MPV_DLL}`);
  process.exit(0);
}

// ---------- 找 asset ----------
const REPOS = ["shinchiro/mpv-winbuild-cmake", "zhongfly/mpv-winbuild"];
let asset = null;
const want = process.env.XVIDEO_MPV_ASSET || null;

for (const repo of REPOS) {
  if (asset) break;
  try {
    const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
      headers: headers(),
    });
    if (!res.ok) {
      console.warn(`[mpv] ${repo} API ${res.status}（${await res.text().then((t) => t.slice(0, 200))}）`);
      continue;
    }
    const release = await res.json();
    const candidates = (release.assets || [])
      .filter((a) => /^mpv-dev-x86_64-\d{8}-git-.+\.7z$/.test(a.name))
      .sort((a, b) => b.name.localeCompare(a.name));
    console.log(`[mpv] ${repo} tag=${release.tag_name} 候选 ${candidates.length} 个`);
    asset = want ? candidates.find((a) => a.name === want) || null : candidates[0] || null;
    if (asset) console.log(`[mpv] 选定 ${asset.name} (${(asset.size / 1048576).toFixed(1)} MB)`);
  } catch (e) {
    console.warn(`[mpv] ${repo} 获取失败：${e.message}`);
  }
}

if (!asset) throw new Error("没有找到可用的 mpv-dev 构建产物");

// ---------- 下载 ----------
mkdirSync(outDir, { recursive: true });
const archive = join(outDir, asset.name);
await downloadTo(asset.browser_download_url, archive);

// ---------- 解压 ----------
function extractWith(cmd, args) {
  try {
    execFileSync(cmd, args, { stdio: ["ignore", "ignore", "pipe"] });
    return existsSync(outFile);
  } catch (e) {
    console.warn(`[mpv] ${cmd} 解压失败：${String(e.stderr || e.message).slice(0, 300)}`);
    return false;
  }
}

const sevenZipCandidates = ["7z", "7z.exe", "C:/Program Files/7-Zip/7z.exe"];

let ok = extractWith("tar", ["-xf", archive, "-C", outDir]);
if (!ok) {
  for (const sz of sevenZipCandidates) {
    ok = extractWith(sz, ["x", archive, `-o${outDir}`, "libmpv-2.dll", "-y"]);
    if (ok) break;
  }
}
if (!ok) {
  // 现下一个 7zr.exe（7-Zip 官方独立解压器，仅支持 7z 格式）
  const seven = join(outDir, "7zr.exe");
  await downloadTo("https://www.7-zip.org/a/7zr.exe", seven);
  ok = extractWith(seven, ["x", archive, `-o${outDir}`, "libmpv-2.dll", "-y"]);
}

if (!existsSync(outFile)) {
  console.error(`[mpv] 目录内容：${readdirSync(outDir).join(", ")}`);
  throw new Error("解压后没有 libmpv-2.dll");
}
if (statSync(outFile).size < 5 * 1024 * 1024) {
  throw new Error(`libmpv-2.dll 大小异常：${statSync(outFile).size} 字节`);
}

try {
  unlinkSync(archive);
} catch {}
console.log(`[mpv] 内核就绪：${outFile}`);
