// 打包前拉取 libmpv 内核（Windows x64）。
// 来源：shinchiro / zhongfly 的 mpv-winbuild 自动构建，只取 mpv-dev 包里的
// libmpv-2.dll。可用 XVIDEO_MPV_ASSET 指定固定版本，便于复现构建。
import { createWriteStream, existsSync, mkdirSync, readdirSync } from "node:fs";
import { createReadStream } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "../src-tauri/resources/mpv");
const outFile = join(outDir, "libmpv-2.dll");

if (process.env.XVIDEO_MPV_DLL && existsSync(process.env.XVIDEO_MPV_DLL)) {
  mkdirSync(outDir, { recursive: true });
  await pipeline(
    createReadStream(process.env.XVIDEO_MPV_DLL),
    createWriteStream(outFile)
  );
  console.log(`[mpv] 使用本地内核 ${process.env.XVIDEO_MPV_DLL}`);
  process.exit(0);
}

const REPOS = [
  "shinchiro/mpv-winbuild-cmake",
  "zhongfly/mpv-winbuild",
];

async function fetchJson(url) {
  const res = await fetch(url, {
    headers: { "user-agent": "x-video-build", accept: "application/vnd.github+json" },
  });
  if (!res.ok) throw new Error(`${url} -> ${res.status}`);
  return res.json();
}

let asset = null;
const want = process.env.XVIDEO_MPV_ASSET || null;

for (const repo of REPOS) {
  try {
    const release = await fetchJson(`https://api.github.com/repos/${repo}/releases/latest`);
    const candidates = (release.assets || []).filter((a) =>
      /^mpv-dev-x86_64-\d{8}-git-.+\.7z$/.test(a.name)
    );
    if (candidates.length === 0) continue;
    candidates.sort((a, b) => b.created_at.localeCompare(a.created_at));
    asset = want
      ? candidates.find((a) => a.name === want) || null
      : candidates[0];
    if (asset) {
      console.log(`[mpv] ${repo} → ${asset.name} (${(asset.size / 1048576).toFixed(1)} MB)`);
      break;
    }
  } catch (e) {
    console.warn(`[mpv] ${repo} 获取失败：${e.message}`);
  }
}

if (!asset) {
  console.error("[mpv] 没有找到可用的 mpv-dev 构建产物");
  process.exit(1);
}

mkdirSync(outDir, { recursive: true });
const tmp = join(outDir, asset.name);
console.log(`[mpv] 下载 ${asset.browser_download_url}`);
await pipeline(Readable.fromWeb((await fetch(asset.browser_download_url)).body), createWriteStream(tmp));

// Windows 自带 bsdtar，可以解 7z
execFileSync("tar", ["-xf", tmp, "-C", outDir], { stdio: "inherit" });

const dll = readdirSync(outDir).find((f) => /^libmpv-2\.dll$/i.test(f));
if (!dll) {
  console.error("[mpv] 解压后没有 libmpv-2.dll：" + readdirSync(outDir).join(", "));
  process.exit(1);
}
console.log(`[mpv] 内核就绪：${outFile}`);
