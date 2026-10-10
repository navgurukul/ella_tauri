import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { layOut, TopicsScreen, type Band, type PlacedCard } from "./TopicsScreen";
import { offeredTopics } from "../lib/topics";

const LEVELS = ["A0", "A1", "A2", "B1", "B2", "C1"];

/** Every level's topics, on a run of days and with a few talked about. */
function lists() {
  return LEVELS.flatMap((level) =>
    [0, 1, 2, 3, 4, 5, 6, 7].map((day) => offeredTopics(level, day % 3 === 0 ? [] : ["street-food", "a1-hobbies", "b1-stress"], 20_000 + day)),
  );
}

/** Which grid cells of its band a card covers, as "row:col". */
function cells(card: PlacedCard): string[] {
  const covered = [];
  for (let row = card.row; row < card.row + card.rows; row += 1) {
    for (let col = card.col; col < card.col + card.cols; col += 1) covered.push(`${row}:${col}`);
  }
  return covered;
}

function touch(a: PlacedCard, b: PlacedCard): boolean {
  const meets = (start: number, span: number, otherStart: number, otherSpan: number) =>
    start < otherStart + otherSpan && otherStart < start + span;
  return (
    ((a.col + a.cols === b.col || b.col + b.cols === a.col) && meets(a.row, a.rows, b.row, b.rows)) ||
    ((a.row + a.rows === b.row || b.row + b.rows === a.row) && meets(a.col, a.cols, b.col, b.cols))
  );
}

describe("All topics", () => {
  it("lays every topic out once, in Home's order, today's talk leading as the tall card", () => {
    for (const topics of lists()) {
      const { bands } = layOut(topics);
      const placed = bands.flatMap((band) => band.cards.map((card) => card.topic.id));
      expect(placed).toEqual(topics.map((topic) => topic.id));
      // Home's own grid: the tall card on the left, the wide one under the rest.
      expect(bands[0].cards[0]).toMatchObject({ slot: "tall", rows: 2, col: 1 });
      expect(bands[0].cards[3]).toMatchObject({ slot: "wide", row: 2 });
    }
  });

  it("fills every row of a band, twelve columns across, with no card over another", () => {
    for (const topics of lists()) {
      for (const band of layOut(topics).bands) {
        const covered = band.cards.flatMap(cells);
        expect(new Set(covered).size).toBe(covered.length);
        expect(covered).toHaveLength(12 * band.rows.length);
        for (const card of band.cards) {
          expect(card.col + card.cols - 1).toBeLessThanOrEqual(12);
          expect(card.row + card.rows - 1).toBeLessThanOrEqual(band.rows.length);
        }
      }
    }
  });

  it("never draws two bands in a row the same shape, and keeps the strip for one left over", () => {
    for (const topics of lists()) {
      const { bands } = layOut(topics);
      bands.forEach((band: Band, index) => {
        if (index > 0) expect(band.shape).not.toBe(bands[index - 1].shape);
        if (band.shape === "strip") expect(index).toBe(bands.length - 1);
      });
    }
  });

  it("never colours two cards that touch the same, across bands too", () => {
    for (const topics of lists()) {
      const { bands } = layOut(topics);
      bands.forEach((band, index) => {
        for (const card of band.cards) {
          for (const other of band.cards) {
            if (other !== card && touch(card, other)) expect(other.tone).not.toBe(card.tone);
          }
          if (index === 0 || card.row !== 1) continue;
          const above = bands[index - 1];
          for (const upper of above.cards) {
            const atFoot = upper.row + upper.rows === above.rows.length + 1;
            const meets = upper.col < card.col + card.cols && card.col < upper.col + upper.cols;
            if (atFoot && meets) expect(upper.tone).not.toBe(card.tone);
          }
        }
      });
    }
  });

  it("draws the same layout for the same topics, and a new one when they change", () => {
    const topics = offeredTopics("A2", [], 20_000);
    expect(layOut(topics)).toEqual(layOut(topics));
    const shapes = (day: number) => layOut(offeredTopics("A2", [], day)).bands.map((band) => band.shape).join();
    const drawn = new Set([20_000, 20_001, 20_002, 20_003, 20_004, 20_005].map(shapes));
    expect(drawn.size).toBeGreaterThan(1);
  });

  it("starts the topic picked, and goes back Home", () => {
    const topics = offeredTopics("A1", [], 20_000);
    const onStart = vi.fn();
    const onBack = vi.fn();
    render(<TopicsScreen topics={topics} busy={false} onBack={onBack} onStart={onStart} />);
    fireEvent.click(screen.getByRole("button", { name: /my hobbies/i }));
    expect(onStart).toHaveBeenCalledWith(topics.find((topic) => topic.id === "a1-hobbies"));
    fireEvent.click(screen.getByRole("button", { name: "Back to Home" }));
    expect(onBack).toHaveBeenCalled();
  });
});
