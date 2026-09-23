/**
 * Pictures, in both of the directions they travel.
 *
 * Out: a picture on its way from the phone to a session is scaled here, since
 * the phone is the only place that can cheaply make the file smaller. A Pixel
 * screenshot is 1080×2400, a photograph 4080×3072; neither is what the model reads.
 *
 * In: a picture a session pointed at names the Mac's LAN address or a path on
 * its disk, and the phone, on a one-way VPN, can reach neither. [[pictorial]]
 * decides which links get rewritten to the console and [[fetchedAt]] does it;
 * `console::images::fetch` is the other end.
 */

/**
 * The longest edge worth sending — Anthropic's own figure; above roughly 1568px
 * an image is scaled down at the far end anyway.
 */
export const LONGEST = 1568;

/** A picture, as the runner's `/image` route takes it. */
export interface Picture {
  /** Bare base64 — no `data:` prefix; the CLI wants it as the API defines it. */
  readonly data: string;
  readonly mediaType: string;
  readonly width: number;
  readonly height: number;
  /** What it weighs on the wire, for the line that says so before it is sent. */
  readonly bytes: number;
  /** For the preview, and revoked when the picture is dropped. */
  readonly preview: string;
}

/**
 * The size to draw at: the same shape, no longer than [LONGEST] on either edge,
 * and never enlarged.
 */
export function fitted(
  width: number,
  height: number,
  longest = LONGEST,
): { width: number; height: number } {
  const scale = Math.min(1, longest / Math.max(width, height));
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

/**
 * Scale a chosen file to something worth sending. Encoded twice and the smaller
 * wins: a screenshot compresses far better as PNG, a photograph several times
 * smaller as JPEG, and guessing from the source type gets the
 * screenshot-photographed-as-JPEG case wrong.
 */
export async function shrink(file: File): Promise<Picture> {
  const bitmap = await createImageBitmap(file);
  const size = fitted(bitmap.width, bitmap.height);
  const canvas = document.createElement('canvas');
  canvas.width = size.width;
  canvas.height = size.height;
  const brush = canvas.getContext('2d');
  if (!brush) throw new Error('this browser would not give a canvas to draw on');
  brush.drawImage(bitmap, 0, 0, size.width, size.height);
  bitmap.close();

  const candidates = await Promise.all([
    encoded(canvas, 'image/png'),
    encoded(canvas, 'image/jpeg', 0.85),
  ]);
  const chosen = candidates.reduce((best, one) => (one.size < best.size ? one : best));

  return {
    data: await base64(chosen),
    mediaType: chosen.type,
    width: size.width,
    height: size.height,
    bytes: chosen.size,
    preview: URL.createObjectURL(chosen),
  };
}

/** One encoding of what is on the canvas. */
function encoded(canvas: HTMLCanvasElement, type: string, quality?: number): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error(`this browser cannot write ${type}`))),
      type,
      quality,
    );
  });
}

/**
 * The blob as bare base64, through a FileReader: walking a megabyte through
 * `String.fromCharCode` blocks the main thread visibly.
 */
function base64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('the picture could not be read'));
    reader.onload = () => {
      // `readAsDataURL` gives a string or nothing; the ArrayBuffer arm cannot happen here.
      const url = typeof reader.result === 'string' ? reader.result : '';
      if (!url) {
        reject(new Error('the picture read back as nothing'));
        return;
      }
      const comma = url.indexOf(',');
      // `data:image/png;base64,` — the runner wants what follows it.
      resolve(comma < 0 ? url : url.slice(comma + 1));
    };
    reader.readAsDataURL(blob);
  });
}

/** How big it is, in the words a person uses about a photo. */
export function weight(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  // Bytes below a kilobyte: `0 kB` beside a thumbnail reads as a picture that
  // failed to load.
  return bytes >= 1024 ? `${Math.round(bytes / 1024)} kB` : `${bytes} B`;
}

/**
 * The endings that make a link worth opening as a picture. An extension, not a
 * probe: a transcript that fetched every link to find out would reach out to
 * whatever a session quoted, on scroll. SVG is left out — it carries script.
 */
const SHOWN = ['.png', '.jpg', '.jpeg', '.gif', '.webp'];

/**
 * Whether the console could open this at all — an address, when the session
 * was also running a server, or a path, which is the commoner shape: a browser
 * resolves `/Users/…/x.jpg` against the console's own origin, where it falls
 * through to this app. `/api/…` is not a path on a disk.
 */
export function fetchable(href: string): boolean {
  if (href.startsWith('/api/')) return false;
  // A path, which `new URL` cannot parse without a base: what makes it openable
  // is that it names a file on the machine the console is on.
  if (href.startsWith('/')) return true;
  let asked: URL;
  try {
    asked = new URL(href);
  } catch {
    return false;
  }
  // `file:` is the path shape with a scheme on it; `coach` writes
  // `[caption](file:///Volumes/…/x.png)`, which the console serves. Not re-checked
  // here: `images::fetch` refuses a
  // `file:` URL with a host, and the bound that counts is where bytes are read.
  return asked.protocol === 'http:' || asked.protocol === 'https:' || asked.protocol === 'file:';
}

/** Whether a link points at something this app can open as a picture. */
export function pictorial(href: string): boolean {
  if (!fetchable(href)) return false;
  // The path alone, so a search for `cat.png` does not qualify and a render with a
  // query of its own still does. The placeholder base is never fetched from.
  const path = new URL(href, 'http://console.invalid').pathname.toLowerCase();
  return SHOWN.some((ending) => path.endsWith(ending));
}

/**
 * Where to ask the console for a picture that lives somewhere else.
 * `encodeURIComponent`: the URL routinely holds `?`, `&` and `#` of its own.
 */
export function fetchedAt(href: string): string {
  return `${WHERE}?url=${encodeURIComponent(href)}`;
}

/** The console's route for a picture that lives somewhere else. */
const WHERE = '/api/picture';

/**
 * The address a picture link was rewritten from, or nothing if it is not one —
 * the inverse of [[fetchedAt]]; a tap has the DOM and nothing else. Parsed
 * rather than sliced: the `url` parameter is percent-encoded and may carry a
 * query. The base is a placeholder, so this is testable outside a page.
 */
export function pointedAt(href: string): string | undefined {
  let asked: URL;
  try {
    asked = new URL(href, 'http://console.invalid');
  } catch {
    return undefined;
  }
  if (asked.pathname !== WHERE) return undefined;
  return asked.searchParams.get('url') ?? undefined;
}
