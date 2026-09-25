/**
 * The colour of the learner's little blob avatar. It is kept on the learner by
 * the backend (`save_avatar_color`), so it stays with them through log out and
 * log in, and greets them on the welcome-back step. A learner who has not
 * picked one, or has one this palette no longer offers, is purple.
 *
 * Earlier versions kept the colour in the webview's storage for the whole
 * device. The helpers at the bottom read that old value once, so it can be
 * moved onto the learner who picked it, and then forget it.
 */
import type { Learner } from "../types";

export const AVATAR_COLORS = [
  { value: "#9347DD", name: "Purple" },
  { value: "#FF3181", name: "Pink" },
  { value: "#FF7A00", name: "Orange" },
  { value: "#68B506", name: "Green" },
] as const;

export const DEFAULT_AVATAR_COLOR = AVATAR_COLORS[0].value;

/** One of the palette's colours, whatever case it was stored in, or nothing. */
function paletteColor(value: string | null | undefined): string | undefined {
  if (!value) return undefined;
  return AVATAR_COLORS.find((color) => color.value.toLowerCase() === value.toLowerCase())?.value;
}

/** The colour to draw this learner in. */
export function avatarColorFor(learner: Pick<Learner, "avatar_color"> | null | undefined): string {
  return paletteColor(learner?.avatar_color) ?? DEFAULT_AVATAR_COLOR;
}

const legacyStorageKey = "ella-avatar-color";

/** The device-wide colour an earlier version saved, if it is still there and
 * still one of the palette's. Storage can be missing or refuse to be read;
 * then there is simply nothing to move. */
export function legacyAvatarColor(): string | undefined {
  try {
    return paletteColor(window.localStorage.getItem(legacyStorageKey));
  } catch {
    return undefined;
  }
}

/** Drops the device-wide colour once it has been moved onto a learner, so it
 * is never handed to anyone else. */
export function forgetLegacyAvatarColor(): void {
  try {
    window.localStorage.removeItem(legacyStorageKey);
  } catch {
    // Nothing to forget.
  }
}
