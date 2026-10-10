/**
 * What Ella talks about, as the window and the browser preview need it.
 *
 * The backend owns the topics: it reads the same `shared/topics.json`, offers
 * the learner the ones written for their level in Home's order, and sends them
 * in `AppSnapshot.topics`. The window looks a topic up here only when it is
 * not among those, such as the talk a badge points to. The browser preview has
 * no backend, so it orders them itself, the way `topics::offered` does.
 */
import catalogue from "../../shared/topics.json";
import type { Topic } from "../types";

/** A topic as the catalogue keeps it: the window's fields, and what offers it. */
interface CatalogueTopic {
  id: string;
  label: string;
  kind: string;
  minutes: number;
  meta: string;
  blurb: string;
  greeting?: string;
  opener: string;
  levels: string[];
  min_age?: number;
}

const TOPICS: CatalogueTopic[] = catalogue.topics;

/** How many recent topics Home holds back before offering them again. */
export const RECENT_TOPICS = 20;

function shown(topic: CatalogueTopic): Topic {
  const { id, label, kind, minutes, meta, blurb, opener } = topic;
  return { id, label, kind, minutes, meta, blurb, opener };
}

/** Any topic in the catalogue, whatever level it is written for. */
export function catalogueTopic(id: string): Topic | undefined {
  const topic = TOPICS.find((candidate) => candidate.id === id);
  return topic ? shown(topic) : undefined;
}

/** Ella's first line on a topic: its greeting with the learner's name, then
 * its opener. Mirrors `TopicEntry::opening`. */
export function openingFor(topicId: string, name: string): string {
  const topic = TOPICS.find((candidate) => candidate.id === topicId);
  if (!topic) return `Hi ${name}! What would you like to tell me about today?`;
  const greeting = (topic.greeting ?? catalogue.defaults.greeting).replace("{name}", name);
  return `${greeting} ${topic.opener}`;
}

/**
 * Every topic for a learner at `level`, in Home's order. Mirrors
 * `topics::offered`: the ones not talked about lately first, in catalogue
 * order turned by one place a day, then the `recent` ones (newest first) from
 * the one talked about longest ago, and last of all any the learner is too
 * young for.
 */
export function offeredTopics(level: string, recent: string[], day: number, age?: number | null): Topic[] {
  const atLevel = TOPICS.filter((topic) => topic.levels.includes(level));
  const pool = atLevel.length > 0 ? atLevel : TOPICS;
  const fresh = pool.filter((topic) => !recent.includes(topic.id));
  const turn = fresh.length === 0 ? 0 : ((day % fresh.length) + fresh.length) % fresh.length;
  const stale = [...recent]
    .reverse()
    .map((id) => pool.find((topic) => topic.id === id))
    .filter((topic): topic is CatalogueTopic => topic !== undefined);
  const ordered = [...fresh.slice(turn), ...fresh.slice(0, turn), ...stale];
  const sunk =
    age == null
      ? ordered
      : [
          ...ordered.filter((topic) => (topic.min_age ?? 0) <= age),
          ...ordered.filter((topic) => (topic.min_age ?? 0) > age),
        ];
  return sunk.map(shown);
}

/** Days since 1 January 1970 for the local calendar day of `date`, which is
 * the day the backend turns Home on. */
export function dayNumber(date: Date): number {
  return Math.round(
    (Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) - Date.UTC(1970, 0, 1)) / 86_400_000,
  );
}
