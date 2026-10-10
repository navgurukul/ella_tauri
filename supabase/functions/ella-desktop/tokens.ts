/**
 * Install tokens, as Ella Mobile's device tokens: 32 random bytes in hex,
 * kept only as their SHA-256, so a copy of the database hands out nothing.
 */

export function mintToken(): string {
  return hex(crypto.getRandomValues(new Uint8Array(32)));
}

export async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return hex(new Uint8Array(digest));
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** The token in an `Authorization: Bearer <token>` header, or `null`. */
export function bearer(request: Request): string | null {
  const value = request.headers.get("authorization");
  if (value === null || !value.startsWith("Bearer ")) return null;
  const token = value.slice("Bearer ".length).trim();
  return /^[0-9a-f]{64}$/.test(token) ? token : null;
}
