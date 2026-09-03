/**
 * Pictures, in both of the directions they travel.
 *
 * ## Out: a picture on its way from the phone to a session
 *
 * Everything on that path happens before the upload, because the phone is the
 * only place that can cheaply make the file smaller and the only place that
 * knows the connection it is going over. A Pixel screenshot is 1080×2400 and a
 * photograph is 4080×3072; neither is what the model reads.
 *
 * ## In: a picture a session pointed at
 *
 * A session that renders something serves it and writes the URL into the
 * conversation — observe's reconstruction previews arrive that way. Those URLs
 * name the Mac's LAN address, and the phone reading them is not on that LAN: it
 * reaches the console down a tunnel, and the one-way VPN means nothing routes
 * back. Following such a link from the phone leaves the app (the shell hands any
 * host it does not know to the browser) and arrives nowhere.
 *
 * So the link is rewritten to point at the console, which is on that LAN and is
 * already being talked to. [[pictorial]] decides which links get that treatment
 * and [[fetchedAt]] does the rewriting; `console::images::fetch` is the other
 * end of it.
 */

/**
 * The longest edge worth sending.
 *
 * ⚠ **Anthropic's own figure**, not a guess: above roughly 1568px on the long
 * edge an image is scaled down at the far end anyway, so everything past it is
 * time on the wire and nothing else. A Pixel screenshot's 2400px height comes to
 * this and loses nothing anybody can read.
 */
export const LONGEST = 1568;

/** A picture, as the runner's `/image` route takes it. */
export interface Picture {
  /** Bare base64 — no `data:` prefix. The runner hands it to the CLI, which
   *  wants it exactly as the API defines it. */
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
 * The size to draw at: the same shape, no longer than [LONGEST] on either edge.
 * Never enlarged — a 400px screenshot blown up to 1568 is the same picture and
 * four times the bytes.
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
 * Scale a chosen file to something worth sending.
 *
 * ⚠ **Encoded twice and the smaller one wins**, which is one rule instead of a
 * threshold. The two formats fail on opposite material: a screenshot of a page is
 * flat colour and compresses far better as PNG, where a photograph of anything
 * real is several times smaller as JPEG. Guessing from the source type gets the
 * screenshot-photographed-as-JPEG case wrong, and a quality knob would be a
 * setting nobody can evaluate without sending both anyway.
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
 * The blob as bare base64.
 *
 * Through a FileReader rather than by walking the bytes: a megabyte read one
 * character at a time through `String.fromCharCode` blocks the phone's main
 * thread for long enough to be seen, and this is the browser's own path.
 */
function base64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('the picture could not be read'));
    reader.onload = () => {
      // `readAsDataURL` gives a string or nothing; the ArrayBuffer arm of this
      // union belongs to the other read methods and cannot happen here.
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
  // Bytes below a kilobyte, because `0 kB` beside a thumbnail reads as a
  // picture that failed to load rather than as a very small one.
  return bytes >= 1024 ? `${Math.round(bytes / 1024)} kB` : `${bytes} B`;
}

/**
 * The endings that make a link worth opening as a picture.
 *
 * ⚠ **An extension and not a probe.** Nothing here can know what a URL is
 * without fetching it, and a transcript that fetched every link to find out
 * would reach out to whatever a session had quoted, on scroll, without anybody
 * asking for it. The endings are what these links actually are — a render
 * server names its files — and a link that is a picture without saying so opens
 * in the browser as it does today, which is the behaviour it already had.
 *
 * SVG is left out deliberately, and the console would refuse it anyway: it is
 * the one image format that carries script.
 */
const SHOWN = ['.png', '.jpg', '.jpeg', '.gif', '.webp'];

/**
 * Whether the console could open this at all — either shape a session writes.
 *
 * **An address**, when the session was also running a server, or **a path**,
 * when it simply has the file. The second is the commoner one and was the one
 * that did not work: observe wrote
 * `![Photo: cabinet corner](/Users/…/lroom-at20s-photo-upright.jpg)`, which a
 * browser resolves against the console's own origin, where it falls through to
 * this very app. `console::images::fetch` reads it off the disk instead.
 *
 * ⚠ **The console's own routes are not paths on a disk.** `/api/…` is this
 * app talking to its runner, and rewriting one of those would send the console
 * to fetch itself.
 */
export function fetchable(href: string): boolean {
  if (href.startsWith('/api/')) return false;
  // A path, which `new URL` cannot parse without a base and should not: what
  // makes it openable is that it names a file on the machine the console is on.
  if (href.startsWith('/')) return true;
  let asked: URL;
  try {
    asked = new URL(href);
  } catch {
    return false;
  }
  // ⚠ **`file:` is the path shape with a scheme on it, and leaving it out cost
  // a whole session's pictures.** `coach` writes
  // `[caption](file:///Volumes/…/soft_squat_left.png)` — a markdown link, so
  // `marked` renders a real anchor, the shell hands the unknown scheme to
  // Chrome, and Chrome has neither a route to this Mac nor any business reading
  // its disk. The console can read it, and already does for the identical path
  // written bare; the scheme was the whole difference (memview#1373).
  //
  // Not re-checked here: `images::fetch` refuses a `file:` URL with a host,
  // because `file://elsewhere/x.png` is another machine. The bound that counts
  // is the one where bytes are actually read, and a second copy of it in the
  // browser would be a rule to keep in step for nothing.
  return asked.protocol === 'http:' || asked.protocol === 'https:' || asked.protocol === 'file:';
}

/** Whether a link points at something this app can open as a picture. */
export function pictorial(href: string): boolean {
  if (!fetchable(href)) return false;
  // ⚠ **The path alone**, so a search for `cat.png` does not qualify and a
  // render carrying a query of its own still does. A bare path has no query to
  // separate, and `new URL` needs a base to say so — hence the placeholder,
  // which nothing is ever fetched from.
  const path = new URL(href, 'http://console.invalid').pathname.toLowerCase();
  return SHOWN.some((ending) => path.endsWith(ending));
}

/**
 * Where to ask the console for a picture that lives somewhere else.
 *
 * ⚠ **`encodeURIComponent` and not a template on its own.** The URL is going
 * into a query parameter and routinely holds `?`, `&` and `#` of its own — a
 * render named `peekA-350-view_top_down.png?again=2` would otherwise arrive at
 * the console as two parameters, and the second half would be silently lost.
 */
export function fetchedAt(href: string): string {
  return `${WHERE}?url=${encodeURIComponent(href)}`;
}

/** The console's route for a picture that lives somewhere else. */
const WHERE = '/api/picture';

/**
 * The address a picture link was rewritten from, or nothing if it is not one.
 *
 * The inverse of [[fetchedAt]], and it exists because the mark and the address
 * both live in the anchor: a tap has the DOM and nothing else. Parsed rather
 * than sliced — the `url` parameter is percent-encoded and may carry a query of
 * its own, which any hand-written split gets wrong.
 *
 * ⚠ **The base is a placeholder, not where anything is fetched from.** `new URL`
 * needs one to parse a relative href at all; only the path and the query are
 * read back out, so what it is cannot matter — and taking it from `location`
 * would make this untestable outside a page for no gain.
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
