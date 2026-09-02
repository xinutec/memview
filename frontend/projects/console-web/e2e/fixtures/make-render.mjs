// The picture the zoom tests are driven against, and how it was made.
//
//     node e2e/fixtures/make-render.mjs
//
// ⚠ **A binary in a repository cannot say what it is for**, and `tiny.png` beside
// it is the reason this exists: at 2×4 it is smaller than the phone even at the
// closest magnification, so a picture made from it has nowhere to be dragged to
// and the pan assertion reads zero against a view that is behaving perfectly.
// The zoom tests need something bigger than the screen; this writes it, and this
// file is the provenance the PNG cannot carry itself.
//
// Eight bands rather than one colour, so a screenshot of a magnified corner says
// which corner it is. 800×1000 is about a render from observe, and compresses to
// a couple of kilobytes.
//
// Here rather than in the spec: the e2e project is typed without Node's library,
// on purpose (see `tinyPng` in ui-pages.spec.ts), so `Buffer` and `node:zlib` in
// a `.ts` file there are 43 lint errors about types that cannot be resolved.

import { writeFileSync } from 'node:fs';
import { crc32, deflateSync } from 'node:zlib';

const WIDTH = 800;
const HEIGHT = 1000;

const BANDS = [
  [0xe6, 0x39, 0x46],
  [0xf2, 0x8f, 0x3b],
  [0xf5, 0xd0, 0x42],
  [0x6a, 0xbf, 0x4b],
  [0x3d, 0x9b, 0xc7],
  [0x3f, 0x51, 0xb5],
  [0x8e, 0x4d, 0xb8],
  [0x55, 0x55, 0x55],
];

// Raw scanlines, each behind the filter byte PNG puts in front of every row.
const raw = Buffer.alloc(HEIGHT * (1 + WIDTH * 3));
for (let y = 0; y < HEIGHT; y++) {
  const row = y * (1 + WIDTH * 3);
  const band = BANDS[Math.min(Math.floor((y / HEIGHT) * BANDS.length), BANDS.length - 1)];
  for (let x = 0; x < WIDTH; x++) {
    raw[row + 1 + x * 3] = band[0];
    raw[row + 2 + x * 3] = band[1];
    raw[row + 3 + x * 3] = band[2];
  }
}

/** One PNG chunk: length, kind, body, and a CRC over the kind and the body. */
function chunk(kind, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(kind, 4, 'ascii');
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])), 0);
  return Buffer.concat([head, body, tail]);
}

const header = Buffer.alloc(13);
header.writeUInt32BE(WIDTH, 0);
header.writeUInt32BE(HEIGHT, 4);
header[8] = 8; // bits per channel
header[9] = 2; // truecolour, no alpha

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', header),
  chunk('IDAT', deflateSync(raw)),
  chunk('IEND', Buffer.alloc(0)),
]);

const at = new URL('./render.png', import.meta.url);
writeFileSync(at, png);
console.log(`wrote ${at.pathname} — ${WIDTH}×${HEIGHT}, ${png.length} bytes`);
