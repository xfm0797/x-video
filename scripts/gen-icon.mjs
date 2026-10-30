// 生成 src-tauri/icons/icon.png 与 icon.ico（无外部依赖，纯 Node 实现）
// 图案：深蓝圆角底 + 白色播放三角
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 256;
const W = SIZE, H = SIZE;

// ---------- PNG ----------
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

// 像素绘制
const px = Buffer.alloc(W * H * 4);
function set(x, y, r, g, b, a = 255) {
  const i = (y * W + x) * 4;
  px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = a;
}
function insideRoundedRect(x, y, x0, y0, x1, y1, r) {
  if (x < x0 || x > x1 || y < y0 || y > y1) return false;
  const cx = Math.max(x0 + r, Math.min(x, x1 - r));
  const cy = Math.max(y0 + r, Math.min(y, y1 - r));
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
}
function insideTriangle(x, y, ax, ay, bx, by, cx, cy) {
  const s = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
  const t = (cx - bx) * (y - by) - (cy - by) * (x - bx);
  const u = (ax - cx) * (y - cy) - (ay - cy) * (x - cx);
  return (s >= 0 && t >= 0 && u >= 0) || (s <= 0 && t <= 0 && u <= 0);
}
for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    if (insideRoundedRect(x, y, 8, 8, 247, 247, 56)) {
      if (insideTriangle(x, y, 96, 72, 96, 184, 192, 128)) {
        set(x, y, 255, 255, 255);
      } else {
        set(x, y, 24, 47, 89); // #182F59
      }
    } else {
      set(x, y, 0, 0, 0, 0);
    }
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0);
ihdr.writeUInt32BE(H, 4);
ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
const raw = Buffer.alloc(H * (1 + W * 4));
for (let y = 0; y < H; y++) {
  raw[y * (1 + W * 4)] = 0;
  px.copy(raw, y * (1 + W * 4) + 1, y * W * 4, (y + 1) * W * 4);
}
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw)),
  chunk("IEND", Buffer.alloc(0)),
]);

// ---------- ICO（内嵌 PNG，256px） ----------
const entry = Buffer.alloc(16);
entry[0] = 0; entry[1] = 0;           // 256px
entry[2] = 0; entry[3] = 0;
entry.writeUInt16LE(1, 4);            // planes
entry.writeUInt16LE(32, 6);           // bitcount
entry.writeUInt32LE(png.length, 8);   // 数据长度
entry.writeUInt32LE(22, 12);          // 数据偏移 = 6 头 + 16 项
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(1, 4);
const ico = Buffer.concat([header, entry, png]);

const outDir = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");
mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "icon.png"), png);
writeFileSync(join(outDir, "icon.ico"), ico);
console.log("icons generated:", outDir);
