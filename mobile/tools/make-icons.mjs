// Regenerates every icon, splash, favicon, notification and in-app logo image from the desktop's
// own icon art: apps/app/assets/icon/source/source.png is a copy of
// desktop/src-tauri/icons/orochi/source.png (white eight-headed serpent, red eyes, black tile).
// The emblem is lifted off the tile by treating the tile's black as transparent, then placed on
// a full-bleed square (iOS), a transparent layer (Android), and so on. Nothing is redrawn.
// Run: `node tools/make-icons.mjs`. Output: apps/app/assets/icon/*.png, e2e/screenshots/icon-sheet.png.
import { mkdirSync, readFileSync, writeFileSync, existsSync, rmSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';
import { PNG } from 'pngjs';

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, '../apps/app/assets/icon');
const sheet = join(here, '../e2e/screenshots/icon-sheet.png');
const desktopIcon = join(here, '../../desktop/src-tauri/icons/orochi/icon.png');
mkdirSync(out, { recursive: true });
for (const f of readdirSync(out)) if (f.endsWith('.svg')) rmSync(join(out, f));

const BLACK = '#121212'; // the desktop icon's tile
const DEEP = '#080808'; // the dark variant, a little deeper

// 1. Lift the emblem: the tile is black, so a pixel's brightest channel is its coverage.
const src = PNG.sync.read(readFileSync(join(out, 'source/source.png')));
const key = (r, g, b) => Math.max(r, g, b);
const covers = new Float32Array(src.width * src.height);
let x0 = src.width,
  y0 = src.height,
  x1 = 0,
  y1 = 0;
for (let y = 0; y < src.height; y++)
  for (let x = 0; x < src.width; x++) {
    const i = (y * src.width + x) * 4;
    const a =
      Math.min(
        1,
        Math.max(0, (key(src.data[i], src.data[i + 1], src.data[i + 2]) - 34) / (255 - 34)),
      ) *
      (src.data[i + 3] / 255);
    covers[y * src.width + x] = a;
    if (a > 0.5) {
      x0 = Math.min(x0, x);
      y0 = Math.min(y0, y);
      x1 = Math.max(x1, x);
      y1 = Math.max(y1, y);
    }
  }
const pad = 6;
x0 -= pad;
y0 -= pad;
x1 += pad;
y1 += pad;
const bw = x1 - x0 + 1,
  bh = y1 - y0 + 1;
function emblem(recolor) {
  const p = new PNG({ width: bw, height: bh });
  for (let y = 0; y < bh; y++)
    for (let x = 0; x < bw; x++) {
      const s = ((y + y0) * src.width + (x + x0)) * 4;
      const a = covers[(y + y0) * src.width + (x + x0)];
      const o = (y * bw + x) * 4;
      if (recolor) [p.data[o], p.data[o + 1], p.data[o + 2]] = recolor;
      else
        for (let k = 0; k < 3; k++)
          p.data[o + k] = a > 0 ? Math.min(255, Math.round(src.data[s + k] / a)) : 0;
      p.data[o + 3] = Math.round(a * 255);
    }
  return PNG.sync.write(p);
}
const art = {
  color: emblem(null),
  white: emblem([255, 255, 255]),
  gray: emblem([200, 200, 200]),
  black: emblem([0, 0, 0]),
};
const b64 = (name) => art[name].toString('base64');

// 2. Compose with resvg (the emblem is an <image>, scaled by its width as a share of the canvas).
const place = (canvas, share, name, dy = 0) => {
  const w = canvas * share;
  const h = (w * bh) / bw;
  return `<image x="${(canvas - w) / 2}" y="${(canvas - h) / 2 + dy * canvas}" width="${w}" height="${h}" href="data:image/png;base64,${b64(name)}"/>`;
};
const svg = (canvas, body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${canvas}" height="${canvas}" viewBox="0 0 ${canvas} ${canvas}">${body}</svg>`;
const rect = (canvas, fill, rx = 0) =>
  `<rect width="${canvas}" height="${canvas}" rx="${rx}" fill="${fill}"/>`;
const S = 1024;
const files = {
  'icon-light': [S, rect(S, BLACK) + place(S, 0.8, 'color')],
  'icon-dark': [S, rect(S, DEEP) + place(S, 0.8, 'color')],
  'icon-tinted': [S, rect(S, '#000') + place(S, 0.8, 'gray')],
  'adaptive-foreground': [S, place(S, 0.54, 'color')],
  'adaptive-background': [S, rect(S, BLACK)],
  'adaptive-monochrome': [S, place(S, 0.54, 'white')],
  splash: [600, place(600, 1, 'color')],
  notification: [96, place(96, 0.96, 'white')],
  favicon: [48, rect(48, BLACK, 10) + place(48, 0.92, 'color')],
  // The in-app logo: the emblem on its own black tile, so it reads on light and dark screens.
  logo: [256, rect(256, BLACK, 56) + place(256, 0.84, 'color')],
};
const png = {};
for (const [name, [canvas, body]] of Object.entries(files)) {
  png[name] = new Resvg(svg(canvas, body), { fitTo: { mode: 'width', value: canvas } })
    .render()
    .asPng();
  // iOS wants no alpha channel at all on the app icon.
  if (name.startsWith('icon-'))
    png[name] = PNG.sync.write(PNG.sync.read(png[name]), {
      colorType: 2,
      inputColorType: 6,
      inputHasAlpha: true,
    });
  writeFileSync(join(out, `${name}.png`), png[name]);
}
for (const f of ['splash-light.png', 'splash-dark.png'])
  if (existsSync(join(out, f))) rmSync(join(out, f));

// 3. The preview sheet.
const img = (data, x, y, w, rx = 0) => {
  const id = `c${Math.random().toString(36).slice(2)}`;
  return `<clipPath id="${id}"><rect x="${x}" y="${y}" width="${w}" height="${w}" rx="${rx}"/></clipPath><image x="${x}" y="${y}" width="${w}" height="${w}" clip-path="url(#${id})" href="data:image/png;base64,${data.toString('base64')}"/>`;
};
const label = (t, x, y) =>
  `<text x="${x}" y="${y}" font-family="Helvetica, Arial, sans-serif" font-size="14" fill="#9aa0a8">${t}</text>`;
const checker = (x0_, y0_, w) => {
  let s = '';
  const n = 8,
    c = w / n;
  for (let i = 0; i < n; i++)
    for (let j = 0; j < n; j++)
      s += `<rect x="${x0_ + i * c}" y="${y0_ + j * c}" width="${c}" height="${c}" fill="${(i + j) % 2 ? '#7a7f86' : '#9aa0a8'}"/>`;
  return s;
};
const desk = existsSync(desktopIcon) ? readFileSync(desktopIcon) : null;
let body = `<rect width="1400" height="1060" fill="#20242a"/><rect x="700" width="700" height="500" fill="#f6f7f9"/>`;
body += label(
  'iOS icon at 29 40 60 87 120 180 px (OS corner radius about 22%), on dark and on light',
  20,
  24,
);
const sizes = [29, 40, 60, 87, 120, 180];
let x = 20;
for (const px of sizes) {
  body += img(png['icon-light'], x, 40, px, px * 0.225);
  x += px + 16;
}
x = 720;
for (const px of sizes) {
  body += img(png['icon-dark'], x, 40, px, px * 0.225);
  x += px + 16;
}
body += label(
  'light | dark | tinted (full-bleed 1024 shown at 200) | desktop icon.png, the source of the art',
  20,
  260,
);
body +=
  img(png['icon-light'], 20, 280, 200, 45) +
  img(png['icon-dark'], 240, 280, 200, 45) +
  img(png['icon-tinted'], 460, 280, 200, 45);
if (desk) body += img(desk, 720, 270, 220);
body += img(png['icon-light'], 960, 280, 200, 45);
body += label(
  'Android: foreground | background | monochrome, with the launcher circle (66% safe zone) | notification 96 and 24',
  20,
  530,
);
for (const [n, lx] of [
  ['adaptive-foreground', 20],
  ['adaptive-background', 250],
  ['adaptive-monochrome', 480],
]) {
  body += checker(lx, 550, 200) + img(png[n], lx, 550, 200);
  body += `<circle cx="${lx + 100}" cy="650" r="66" fill="none" stroke="#ff4d6d" stroke-width="1.5" stroke-dasharray="4 3"/>`;
}
body +=
  `<rect x="710" y="550" width="200" height="200" fill="#7a7f86"/>` +
  img(png.notification, 760, 560, 96) +
  img(png.notification, 880, 560, 24);
body += label('adaptive on circle masks (black, white)', 940, 570);
for (const [i, bg] of [
  [0, BLACK],
  [1, '#ffffff'],
]) {
  body += `<circle cx="${1010 + i * 150}" cy="650" r="66" fill="${bg}"/>`;
  body += `<clipPath id="m${i}"><circle cx="${1010 + i * 150}" cy="650" r="66"/></clipPath>`;
  body += `<g clip-path="url(#m${i})"><rect x="${944 + i * 150}" y="584" width="132" height="132" fill="${BLACK}"/>${img(png['adaptive-foreground'], 944 + i * 150, 584, 132)}</g>`;
}
body += label(
  'splash | favicon 48 32 16 | in-app logo 28 40 84 | mark alone on light and dark at 29 and 20',
  20,
  790,
);
body +=
  rect(1, 'none') +
  `<rect x="20" y="800" width="200" height="200" fill="${BLACK}"/>` +
  img(png.splash, 70, 850, 100);
body +=
  img(png.favicon, 250, 810, 48) + img(png.favicon, 310, 810, 32) + img(png.favicon, 355, 810, 16);
body += img(png.logo, 250, 880, 28) + img(png.logo, 290, 880, 40) + img(png.logo, 340, 880, 84);
body += `<rect x="460" y="800" width="130" height="80" fill="${BLACK}"/><rect x="600" y="800" width="130" height="80" fill="#ffffff"/>`;
body +=
  img(png['adaptive-foreground'], 465, 805, 29 * 2.2) +
  img(png['adaptive-foreground'], 535, 810, 40);
body += img(png.logo, 610, 810, 29) + img(png.logo, 660, 810, 20);
writeFileSync(
  sheet,
  new Resvg(
    svg(1400, body).replace(
      'height="1400" viewBox="0 0 1400 1400"',
      'height="1060" viewBox="0 0 1400 1060"',
    ),
    { fitTo: { mode: 'width', value: 1400 } },
  )
    .render()
    .asPng(),
);
console.log('icons written to', out, 'and', sheet);
