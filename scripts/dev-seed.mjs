// 生成一套用于界面联调的假媒体库（图片 + 占位视频文件）
// 用法：node scripts/dev-seed.mjs [目标目录]
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = process.argv[2] || join(dirname(fileURLToPath(import.meta.url)), "../.devmedia");

const crcTable = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** 画一张渐变海报：两色线性渐变 + 柔光圆 + 底部压暗 */
function poster(w, h, c1, c2, seed) {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  let s = seed;
  const rnd = () => {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    return s / 0x7fffffff;
  };
  const cx = w * (0.25 + rnd() * 0.5);
  const cy = h * (0.2 + rnd() * 0.35);
  const rad = Math.min(w, h) * (0.45 + rnd() * 0.25);

  for (let y = 0; y < h; y++) {
    const row = y * (w * 3 + 1);
    raw[row] = 0;
    for (let x = 0; x < w; x++) {
      const t = (x / w) * 0.55 + (y / h) * 0.45;
      let r = c1[0] + (c2[0] - c1[0]) * t;
      let g = c1[1] + (c2[1] - c1[1]) * t;
      let b = c1[2] + (c2[2] - c1[2]) * t;

      const d = Math.hypot(x - cx, y - cy) / rad;
      if (d < 1) {
        const k = Math.pow(1 - d, 2.2) * 0.55;
        r += (255 - r) * k;
        g += (255 - g) * k;
        b += (255 - b) * k;
      }
      const vign = 1 - Math.pow(y / h, 3) * 0.45;
      const i = row + 1 + x * 3;
      raw[i] = Math.max(0, Math.min(255, r * vign)) | 0;
      raw[i + 1] = Math.max(0, Math.min(255, g * vign)) | 0;
      raw[i + 2] = Math.max(0, Math.min(255, b * vign)) | 0;
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const palettes = [
  [[24, 32, 58], [126, 66, 148]],
  [[10, 44, 52], [40, 148, 158]],
  [[58, 20, 26], [186, 92, 58]],
  [[16, 22, 44], [70, 108, 190]],
  [[44, 30, 60], [200, 120, 90]],
  [[12, 30, 34], [96, 150, 96]],
];

const filler = Buffer.alloc(140 * 1024, 7);

function touch(path) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, filler);
}

function makeSeries(dir, title, count, opts = {}) {
  const base = join(root, dir);
  mkdirSync(base, { recursive: true });
  const pal = palettes[Math.abs(hash(title)) % palettes.length];
  const square = opts.music;
  writeFileSync(
    join(base, "poster.png"),
    poster(square ? 500 : 400, square ? 500 : 600, pal[0], pal[1], Math.abs(hash(title)) + 7)
  );
  for (let i = 1; i <= count; i++) {
    const name = opts.name ? opts.name(i) : `${title}.S01E${String(i).padStart(2, "0")}.1080p.WEB-DL.mkv`;
    touch(join(base, name));
  }
}

function hash(s) {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return h;
}

makeSeries("电影/流浪地球2 (2023)", "流浪地球2", 1, {
  name: () => "流浪地球2.2023.2160p.HDR.WEB-DL.H265.10bit-GRP.mkv",
});
makeSeries("电影/沙丘2 (2024)", "沙丘2", 1, {
  name: () => "沙丘2.2024.1080p.BluRay.x264-SPARKS.mkv",
});
makeSeries("电影/奥本海默 (2023)", "奥本海默", 1, {
  name: () => "奥本海默.2023.2160p.WEB-DL.DDP5.1.Atmos.mkv",
});
makeSeries("电影/坠落的审判 (2023)", "坠落的审判", 1, {
  name: () => "坠落的审判.2023.1080p.BluRay.mkv",
});
makeSeries("剧集/漫长的季节", "漫长的季节", 12, {
  name: (i) => `漫长的季节.S01E${String(i).padStart(2, "0")}.1080p.WEB-DL.mkv`,
});
makeSeries("剧集/三体", "三体", 6, {
  name: (i) => `三体 第${String(i).padStart(2, "0")}集.mp4`,
});
makeSeries("剧集/繁花", "繁花", 6, {
  name: (i) => `繁花.EP${String(i).padStart(2, "0")}.1080p.mkv`,
});
makeSeries("短剧/闪婚后，被大佬宠上天", "闪婚后，被大佬宠上天", 78, {
  name: (i) => `闪婚后，被大佬宠上天-${String(i).padStart(3, "0")}.mp4`,
});
makeSeries("短剧/重生之我在古代当首富", "重生之我在古代当首富", 64, {
  name: (i) => `重生之我在古代当首富-${String(i).padStart(3, "0")}.mp4`,
});
makeSeries("音乐/周深 · 深龄其境", "周深 · 深龄其境", 10, {
  music: true,
  name: (i) => `${String(i).padStart(2, "0")}. 深龄其境.flac`,
});
makeSeries("音乐/久石让 · Summer", "久石让 · Summer", 8, {
  music: true,
  name: (i) => `${String(i).padStart(2, "0")}. Summer.mp3`,
});
makeSeries("音乐/陈奕迅 · U87", "陈奕迅 · U87", 11, {
  music: true,
  name: (i) => `${String(i).padStart(2, "0")}. 浮夸.mp3`,
});

console.log(root);
