/**
 * Share-token storage (the health pattern): a recipient lands on
 * /share/<token>, the token is kept in localStorage, and every API request
 * carries it as X-Share-Token. The owner's session cookie, when present,
 * wins server-side.
 */

const KEY = 'memview_share_token';

export function getShareToken(): string | null {
  return localStorage.getItem(KEY);
}

export function setShareToken(token: string): void {
  localStorage.setItem(KEY, token);
}

export function clearShareToken(): void {
  localStorage.removeItem(KEY);
}
