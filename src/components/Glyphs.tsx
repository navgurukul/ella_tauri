import type { BadgeGlyph } from "../types";

/** Each glyph as the design draws it (its `IC`), on a 24-unit square. */
const GLYPHS: Record<BadgeGlyph, string> = {
  ella: "M7.9 3.04A1.3 1.3 0 0110.41 2.36L11.46 6.26A1.3 1.3 0 018.95 6.94ZM13.59 2.36A1.3 1.3 0 0116.1 3.04L15.05 6.94A1.3 1.3 0 0112.54 6.26ZM3 21v-5.2C3 11.5 7 8.2 12 8.2s9 3.3 9 7.6V21H3zM9.6 13.1a1.3 1.3 0 100 2.6 1.3 1.3 0 000-2.6zM14.4 13.1a1.3 1.3 0 100 2.6 1.3 1.3 0 000-2.6z",
  mic: "M12 3a3 3 0 013 3v5a3 3 0 01-6 0V6a3 3 0 013-3zM6 11a6 6 0 0012 0h2a8 8 0 01-7 7.94V21h-2v-2.06A8 8 0 014 11z",
  tag: "M3 4.6C3 3.7 3.7 3 4.6 3h6.1c.4 0 .8.2 1.1.5l8.7 8.7c.6.6.6 1.6 0 2.3l-6.1 6.1c-.6.6-1.6.6-2.3 0L3.5 11.8c-.3-.3-.5-.7-.5-1.1zM7.6 6a1.6 1.6 0 100 3.2 1.6 1.6 0 000-3.2z",
  house: "M11 3.2a1.6 1.6 0 012 0l7.6 6.2c.5.4.2 1.3-.6 1.3H19v6.1a3.2 3.2 0 01-3.2 3.2H8.2A3.2 3.2 0 015 16.8v-6.1H4c-.8 0-1.1-.9-.6-1.3L11 3.2zm-.6 16.8v-3.2a1.6 1.6 0 013.2 0V20h-3.2z",
  flame: "M12 2.2c.5 2.9 2.2 4.5 3.8 6.1 1.6 1.6 3.1 3.4 3.1 6.2A6.9 6.9 0 0112 21.4a6.9 6.9 0 01-6.9-6.9c0-2.6 1.2-4.6 2.9-6.1.2 1.6.9 2.7 2 3.3-.5-3.5.3-6.8 2-9.5zM12 13.4c-1.5 1.3-2.4 2.5-2.4 3.9a2.4 2.4 0 004.8 0c0-1.4-.9-2.6-2.4-3.9z",
  star: "M12 2.8l2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.1 6.4 20l1.1-6.2L3 9.4l6.2-.9z",
};

export function Glyph({ glyph, size, color }: { glyph: BadgeGlyph; size: number; color: string }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" focusable="false">
      <path d={GLYPHS[glyph]} fill={color} fillRule="evenodd" />
    </svg>
  );
}

/** A step not reached yet: Ella Mobile's padlock, filled. */
export function Lock({ size = 13 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" focusable="false">
      <path
        d="M7 10V7.5a5 5 0 0110 0V10h.5c.8 0 1.5.7 1.5 1.5v8c0 .8-.7 1.5-1.5 1.5h-11c-.8 0-1.5-.7-1.5-1.5v-8c0-.8.7-1.5 1.5-1.5zm2.2 0h5.6V7.5a2.8 2.8 0 00-5.6 0z"
        fill="currentColor"
        fillRule="evenodd"
      />
    </svg>
  );
}

/** The tick the design draws for anything done. */
export function Check({ size = 14 }: { size?: number }) {
  return (
    <svg className="check-glyph" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" focusable="false">
      <path d="M5 12.5l4.2 4.2L19 7" />
    </svg>
  );
}
