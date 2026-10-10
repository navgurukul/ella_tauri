import { describe, expect, it } from "vitest";
import catalogue from "../../shared/topics.json";
import { catalogueTopic, dayNumber, offeredTopics, openingFor } from "./topics";

const ids = (level: string, recent: string[], day: number, age?: number | null) =>
  offeredTopics(level, recent, day, age).map((topic) => topic.id);

describe("the topics", () => {
  it("are Ella Mobile's eighty-three, the desktop's own seven among them", () => {
    expect(catalogue.topics).toHaveLength(83);
    for (const id of ["street-food", "restaurant-order", "booking-a-cab", "job-interview", "doctor-clinic", "asking-directions", "market-bargaining"]) {
      expect(catalogueTopic(id)).toBeDefined();
    }
    expect(catalogueTopic("a0-my-family")).toEqual({
      id: "a0-my-family",
      label: "My family",
      kind: "fluency",
      minutes: 3,
      meta: "FLUENCY · ~3 MIN",
      blurb: "Say who is in your family.",
      opener: "Who is in your family? Tell me their names!",
    });
    expect(catalogueTopic("placement")).toBeUndefined();
  });

  it("open on the greeting, then the opener, as the backend's do", () => {
    expect(openingFor("street-food", "Asha")).toBe(
      "Hi Asha! Tell me about the tastiest thing you ate this week. Where did you find it?",
    );
    expect(openingFor("job-interview", "Asha")).toBe(
      "Hello Asha! Thank you for coming in. To start, could you tell me a little about yourself?",
    );
    expect(openingFor("b2-pitch", "Asha")).toBe(
      "Good morning, Asha. You have two minutes to pitch your idea to our panel. What have you brought us?",
    );
  });

  it("are offered by level, turning one place a day", () => {
    const day = 20_371;
    expect(ids("A0", [], day).every((id) => id.startsWith("a0-"))).toBe(true);
    expect(ids("A2", [], day)).toHaveLength(17);
    const today = ids("A2", [], day);
    // As `topics::offered` has it on the same day.
    expect(today[0]).toBe("a2-when-i-was-little");
    const tomorrow = ids("A2", [], day + 1);
    expect(tomorrow).toEqual([...today.slice(1), today[0]]);
    expect(ids("Z9", [], day)).toHaveLength(83);
    expect(ids("A2", [], -1)).toHaveLength(17);
  });

  it("move a topic just talked about to the back, the latest last", () => {
    const day = 20_371;
    const fresh = ids("A2", [], day);
    const after = ids("A2", [fresh[0], fresh[3], "market-cloth-price"], day);
    expect(after).toHaveLength(fresh.length);
    expect(after.slice(-2)).toEqual([fresh[3], fresh[0]]);
  });

  it("offer a younger learner the grown-up topics last", () => {
    const twelve = ids("B1", [], 3, 12);
    expect(twelve[twelve.length - 1]).toBe("job-interview");
    expect(ids("B1", [], 3, 14)).toEqual(ids("B1", [], 3));
  });

  it("turn on the local calendar day", () => {
    expect(dayNumber(new Date(1970, 0, 1, 0, 5))).toBe(0);
    expect(dayNumber(new Date(2026, 9, 10, 23, 59))).toBe(20_736);
    expect(dayNumber(new Date(2026, 9, 10, 0, 1))).toBe(20_736);
  });
});
