/**
 * A picture on its way from the phone to a session.
 *
 * Everything here happens before the upload, because the phone is the only place
 * that can cheaply make the file smaller and the only place that knows the
 * connection it is going over. A Pixel screenshot is 1080×2400 and a photograph
 * is 4080×3072; neither is what the model reads.
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
