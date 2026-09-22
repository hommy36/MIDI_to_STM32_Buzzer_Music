// 生成 1024x1024 应用图标：深色圆角方底 + accent 色圆 + 白色音符符干
// 用法：node scripts/gen-icon.js  → icons-src.png
const zlib = require("zlib");
const fs = require("fs");

const S = 1024;
const buf = Buffer.alloc(S * S * 4);

const bg = [30, 34, 42];
const accent = [79, 140, 255];
const white = [240, 244, 255];

function setPx(x, y, c) {
  const i = (y * S + x) * 4;
  buf[i] = c[0];
  buf[i + 1] = c[1];
  buf[i + 2] = c[2];
  buf[i + 3] = 255;
}

const cx = S / 2, cy = S / 2, R = S * 0.34;         // 圆
const noteX = S * 0.44, noteY = S * 0.58, noteR = S * 0.085; // 符头
const stemW = S * 0.035, stemTop = S * 0.24;        // 符干

for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    let c = bg;
    const dx = x - cx, dy = y - cy;
    if (dx * dx + dy * dy <= R * R) c = accent;
    // 符头
    const ndx = x - noteX, ndy = y - noteY;
    if (ndx * ndx + ndy * ndy <= noteR * noteR) c = white;
    // 符干
    if (x >= noteX + noteR - stemW && x <= noteX + noteR && y >= stemTop && y <= noteY) c = white;
    // 符尾（旗）
    if (y >= stemTop && y <= stemTop + S * 0.09 && x >= noteX + noteR - stemW && x <= noteX + noteR + S * 0.14) c = white;
    setPx(x, y, c);
  }
}

// PNG 编码
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(body) >>> 0);
  return Buffer.concat([len, body, crc]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8;  // bit depth
ihdr[9] = 6;  // RGBA
const raw = Buffer.alloc(S * (S * 4 + 1));
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0; // filter: none
  buf.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
}
const idat = zlib.deflateSync(raw, { level: 9 });

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", idat),
  chunk("IEND", Buffer.alloc(0)),
]);
fs.writeFileSync("icons-src.png", png);
console.log("icons-src.png written");
