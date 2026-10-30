/**
 * Hashing for the relay. Tokens are minted, read and hashed as
 * `ella-desktop` does (`../ella-desktop/tokens.ts`), and kept in a table of
 * their own: a telemetry token never opens chat, nor a chat token telemetry.
 */

export { bearer, mintToken, sha256 } from "../ella-desktop/tokens.ts";

export async function sha256Hex(data: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Whether two secrets are the same, in a time that does not depend on where
 * they first differ: each is hashed first, so only hashes are compared.
 */
export async function sameSecret(given: string, kept: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [a, b] = await Promise.all([
    sha256Hex(encoder.encode(given)),
    sha256Hex(encoder.encode(kept)),
  ]);
  let difference = 0;
  for (let i = 0; i < a.length; i++) difference |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return difference === 0;
}
