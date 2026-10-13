/**
 * What a new version brought, in words for the learner: what changed for them,
 * never how. shared/whats-new.json holds each version's lines, and is where
 * the release's notes come from too (scripts/release-notes.mjs), so the update
 * card shows the same lines for the version it is about to install
 * (`whatsNewFrom`, out of `latest.json`), and a learner who has just updated
 * is shown this version's own, once (`whatsNewAfterUpdate`).
 */
import { getVersion } from "@tauri-apps/api/app";
import notesByVersion from "../../shared/whats-new.json";

export interface WhatsNew {
  version: string;
  notes: string[];
}

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The version whose notes this window last showed, or that it first ran. */
const SEEN_KEY = "ella-whats-new-seen";

/** What `version` brought, as shared/whats-new.json says it. */
export function whatsNewIn(version: string): string[] {
  return (notesByVersion as Record<string, string[]>)[version] ?? [];
}

/** The lines under the "What's new" heading of a release's notes, as the
 * updater hands them over (`Update.body`). None when the notes have no such
 * heading, as releases before 0.1.21 do not. */
export function whatsNewFrom(body: string | null | undefined): string[] {
  if (!body) return [];
  const lines = body.split(/\r?\n/).map((line) => line.trim());
  const heading = lines.findIndex((line) => /^#{1,3}\s+what['’]s new/i.test(line));
  if (heading < 0) return [];
  const notes: string[] = [];
  for (const line of lines.slice(heading + 1)) {
    if (line.startsWith("#")) break;
    if (/^[-*]\s+/.test(line)) notes.push(line.replace(/^[-*]\s+/, ""));
  }
  return notes.filter(Boolean);
}

/**
 * This version's notes, when the learner has not yet been shown them: once
 * after an update, to someone who used Ella before it (`returning`). A new
 * install shows nothing, and is remembered, so that its first update is
 * shown. Null outside the app, where there is no version to ask, and
 * whenever anything goes wrong: the card is a courtesy, never in the way.
 */
export async function whatsNewAfterUpdate(
  returning: boolean,
  storage: StorageLike = window.localStorage,
): Promise<WhatsNew | null> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return null;
  try {
    const version = await getVersion();
    const seen = storage.getItem(SEEN_KEY);
    if (seen === version) return null;
    if (seen === null && !returning) {
      storage.setItem(SEEN_KEY, version);
      return null;
    }
    const notes = whatsNewIn(version);
    return notes.length > 0 ? { version, notes } : null;
  } catch {
    return null;
  }
}

/** The learner has seen, or put away, what `version` brought. */
export function markWhatsNewSeen(version: string, storage: StorageLike = window.localStorage) {
  try {
    storage.setItem(SEEN_KEY, version);
  } catch {
    // A window that cannot keep it shows the card again next launch.
  }
}
