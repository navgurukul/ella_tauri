import { TopicCard, type TopicSlot } from "./HomeScreen";
import type { Tone, Topic } from "../types";

/**
 * Home's "View all", as on Ella Mobile: every topic at the learner's level, in
 * Home's order, laid out as a bento of Home's own cards in shapes and sizes
 * drawn at random.
 *
 * The phone's bento stacks bands of at most three cards down a narrow page. A
 * band that wide here would make cards half a window across, so a band is
 * twelve columns of Home's own cards instead: Home's grid of a tall card, two
 * small ones and a wide one, or a row of two or three small ones, with a wide
 * card for one left over at the end.
 */
export function TopicsScreen({
  topics,
  busy,
  onBack,
  onStart,
}: {
  topics: Topic[];
  busy: boolean;
  onBack: () => void;
  onStart: (topic: Topic) => void;
}) {
  const { bands } = layOut(topics);

  return (
    <div className="screen screen--scroll screen--topics" data-screen="topics">
      <header className="topics-head">
        <button className="back-button" onClick={onBack} aria-label="Back to Home">
          <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
        <div>
          <h1 className="display page-title">All topics</h1>
          <p className="page-head__sub">Every talk at your level. Pick any one to start.</p>
        </div>
      </header>

      <div className="bento">
        {bands.map((band, index) => (
          <div
            key={index}
            className="bento__band"
            style={{ gridTemplateRows: band.rows.map((row) => `${row}px`).join(" ") }}
          >
            {band.cards.map((card) => (
              <TopicCard
                key={card.topic.id}
                topic={card.topic}
                slot={card.slot}
                tone={card.tone}
                disabled={busy}
                onStart={onStart}
                style={{
                  gridColumn: `${card.col} / span ${card.cols}`,
                  gridRow: `${card.row} / span ${card.rows}`,
                }}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/** Home's colours, then blue, handed out in topic order: a card takes the next
 * in turn unless a card it touches already has it. */
const TONES: Tone[] = ["green", "pink", "orange", "ink", "violet", "blue"];

type Shape = "feature" | "trio" | "pair" | "strip";

/** How many topics each shape holds. A strip only ever takes the last one. */
const SIZE: Record<Shape, number> = { feature: 4, trio: 3, pair: 2, strip: 1 };

/** How tall a row of small cards stands, each tall enough for a three-line
 * title above its meta line; Home's wide card stands lower. */
const SMALL_ROWS = [150, 168, 186];
const WIDE_ROWS = [104, 116];
const STRIP_ROW = 96;

export interface PlacedCard {
  topic: Topic;
  slot: TopicSlot;
  tone: Tone;
  /** Grid lines, from 1, within the card's band. */
  col: number;
  cols: number;
  row: number;
  rows: number;
}

export interface Band {
  shape: Shape;
  /** Each row's height, in pixels. */
  rows: number[];
  cards: PlacedCard[];
}

/**
 * The bento: each band's shape, how it splits the twelve columns and how tall
 * its rows stand, drawn at random. The draw is seeded by the topics, so the
 * layout holds still while the screen draws again, and changes only when the
 * list does.
 *
 * Today's talk leads as the tall card, as it does on Home. After that each
 * band takes a shape the one before did not, drawn only where the topics left
 * can still be laid out that way to the end.
 */
export function layOut(topics: Topic[]): { bands: Band[] } {
  const random = mulberry32(seed(topics));
  const pick = <T,>(from: T[]): T => from[Math.floor(random() * from.length)];

  const bands: Band[] = [];
  let start = 0;
  let last: Shape | null = null;
  while (start < topics.length) {
    const left = topics.length - start;
    const fits = (shape: Shape) =>
      shape !== last && SIZE[shape] <= left && canFinish(left - SIZE[shape], shape);
    let shape: Shape;
    if (last === null && fits("feature")) {
      shape = "feature";
    } else {
      // A pair half as often as the others, since a page of pairs would be a
      // list; a strip for the last topic alone.
      const shapes: Shape[] = left === 1 ? ["strip"] : ["feature", "feature", "trio", "trio", "pair"];
      const weighted = shapes.filter(fits);
      shape = weighted.length > 0 ? pick(weighted) : fallback(left);
    }
    const slice = topics.slice(start, start + SIZE[shape]);
    bands.push(band(shape, slice, pick, random, bands.length === 0));
    start += slice.length;
    last = shape;
  }
  colour(bands);
  return { bands };
}

/** Whether `left` topics can still be laid out after a band of `last`, no two
 * bands in a row the same shape, and a strip only for the very last one. */
function canFinish(left: number, last: Shape): boolean {
  if (left === 0) return true;
  if (left === 1) return last !== "strip";
  return (["feature", "trio", "pair"] as Shape[]).some(
    (shape) => shape !== last && SIZE[shape] <= left && canFinish(left - SIZE[shape], shape),
  );
}

/** A shape that fits when no draw does, which only a list of one to three
 * topics, with nothing before them, can need. */
function fallback(left: number): Shape {
  if (left >= 4) return "feature";
  return left === 3 ? "trio" : left === 2 ? "pair" : "strip";
}

/** One band of `shape`, holding `topics`. The first is laid out as Home's grid
 * is, today's talk the tall card on the left. */
function band(
  shape: Shape,
  topics: Topic[],
  pick: <T>(from: T[]) => T,
  random: () => number,
  first: boolean,
): Band {
  const card = (index: number, slot: TopicSlot, col: number, cols: number, row = 1, rows = 1): PlacedCard => ({
    topic: topics[index],
    slot,
    tone: "green",
    col,
    cols,
    row,
    rows,
  });

  switch (shape) {
    case "feature": {
      // Home's grid: the tall card, two small ones beside it, and a wide one
      // under them; the tall card on either side, the wide one above or below.
      const lead = pick([4, 5]);
      const rest = 12 - lead;
      const split = pick(rest % 2 === 0 ? [rest / 2] : [Math.floor(rest / 2), Math.ceil(rest / 2)]);
      const mirrored = random() < 0.5 && !first;
      const flipped = random() < 0.35 && !first;
      const tallCol = mirrored ? 1 + rest : 1;
      const restCol = mirrored ? 1 : 1 + lead;
      const smallRow = flipped ? 2 : 1;
      const wideRow = flipped ? 1 : 2;
      const small = pick(SMALL_ROWS);
      const wide = pick(WIDE_ROWS);
      return {
        shape,
        rows: flipped ? [wide, small] : [small, wide],
        cards: [
          card(0, "tall", tallCol, lead, 1, 2),
          card(1, "small", restCol, split, smallRow),
          card(2, "small", restCol + split, rest - split, smallRow),
          card(3, "wide", restCol, rest, wideRow),
        ],
      };
    }
    case "trio": {
      const spans = pick([
        [4, 4, 4],
        [3, 5, 4],
        [5, 3, 4],
        [4, 5, 3],
        [3, 4, 5],
        [5, 4, 3],
      ]);
      return { shape, rows: [pick(SMALL_ROWS)], cards: spread(spans, card) };
    }
    case "pair": {
      const spans = pick([
        [5, 7],
        [6, 6],
        [7, 5],
      ]);
      return { shape, rows: [pick(SMALL_ROWS)], cards: spread(spans, card) };
    }
    case "strip":
      return { shape, rows: [STRIP_ROW], cards: [card(0, "wide", 1, 12)] };
  }
}

/** Small cards side by side, `spans` columns each. */
function spread(
  spans: number[],
  card: (index: number, slot: TopicSlot, col: number, cols: number) => PlacedCard,
): PlacedCard[] {
  let col = 1;
  return spans.map((cols, index) => {
    const placed = card(index, "small", col, cols);
    col += cols;
    return placed;
  });
}

/**
 * Gives every card a colour, in topic order: the next in turn, or the first
 * after it that no card it touches has, in its band or the band above. The
 * phone's bands are small enough for the turn alone to keep neighbours apart;
 * Home's grid of four is not, and its wide card can touch six others, so a
 * card left with no colour sends the one before it on to its next. Cards that
 * touch never cross, so six colours always go round.
 */
function colour(bands: Band[]) {
  const cards = bands.flatMap((band, index) => band.cards.map((card) => ({ card, band: index })));
  const earlier = cards.map(({ card, band }, index) =>
    cards.slice(0, index).filter((other) => {
      if (other.band === band) return touches(card, other.card);
      return other.band === band - 1 && onTopOf(other.card, bands[other.band], card);
    }),
  );
  const assign = (index: number): boolean => {
    if (index === cards.length) return true;
    const taken = new Set(earlier[index].map((other) => other.card.tone));
    for (let offset = 0; offset < TONES.length; offset += 1) {
      const tone = TONES[(index + offset) % TONES.length];
      if (taken.has(tone)) continue;
      cards[index].card.tone = tone;
      if (assign(index + 1)) return true;
    }
    return false;
  };
  assign(0);
}

function overlap(start: number, span: number, otherStart: number, otherSpan: number): boolean {
  return start < otherStart + otherSpan && otherStart < start + span;
}

/** Two cards in one band share an edge. */
function touches(card: PlacedCard, other: PlacedCard): boolean {
  const side =
    (card.col + card.cols === other.col || other.col + other.cols === card.col) &&
    overlap(card.row, card.rows, other.row, other.rows);
  const stacked =
    (card.row + card.rows === other.row || other.row + other.rows === card.row) &&
    overlap(card.col, card.cols, other.col, other.cols);
  return side || stacked;
}

/** A card at the foot of the band above sits on top of `card`, at the head of
 * its own band, where their columns meet. */
function onTopOf(above: PlacedCard, aboveBand: Band, card: PlacedCard): boolean {
  return (
    above.row + above.rows === aboveBand.rows.length + 1 &&
    card.row === 1 &&
    overlap(above.col, above.cols, card.col, card.cols)
  );
}

/** The same seed for the same topics in the same order, on every run: FNV-1a
 * over their ids, as the phone seeds its bento. */
function seed(topics: Topic[]): number {
  let hash = 0x811c9dc5;
  for (const topic of topics) {
    for (const unit of [...Array.from(topic.id, (char) => char.charCodeAt(0)), 0]) {
      hash = Math.imul(hash ^ unit, 0x01000193) >>> 0;
    }
  }
  return hash;
}

/** A small seeded generator of numbers in [0, 1). */
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
