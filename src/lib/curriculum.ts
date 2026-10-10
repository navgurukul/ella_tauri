/**
 * The curriculum, as the window and the browser preview need it.
 *
 * The backend owns the curriculum: it reads the same `shared/curriculum.json`,
 * picks the skill each talk aims at, scores talks and moves the learner on. The
 * browser preview has no model to score with, so it only places a learner and
 * draws the ladder — which is what the backend does without a model too.
 */
import curriculum from "../../shared/curriculum.json";
import type { LevelView, Standing } from "../types";

/** A level (its CEFR code, which never reaches the screen) and a step in it. */
export interface Position {
  level: string;
  step: number;
}

/** Where everyone starts until a placement says otherwise: Step 1 of A2. */
export const START: Position = { ...curriculum.start };

export function levelIndex(code: string): number {
  return curriculum.levels.findIndex((level) => level.code === code);
}

/** How many steps a level has; five for a level the curriculum does not have. */
export function stepCount(code: string): number {
  return curriculum.levels.find((level) => level.code === code)?.steps.length ?? 5;
}

/** The name the window shows for a level; an unknown code is shown as it is. */
export function levelName(code: string): string {
  return curriculum.levels.find((level) => level.code === code)?.name ?? code;
}

/**
 * Where a learner at `position` stands with nothing scored yet, as the backend
 * reports it: the steps behind them, and none of this one.
 */
export function standingAt(position: Position, placed: boolean): Standing {
  const index = Math.max(0, levelIndex(position.level));
  const level = curriculum.levels[index];
  const steps = level.steps.length;
  return {
    level_number: index + 1,
    level_count: curriculum.levels.length,
    level_name: level.name,
    step: position.step,
    step_count: steps,
    step_title: level.steps.find((step) => step.number === position.step)?.title ?? "",
    percent: Math.round((100 * (position.step - 1)) / steps),
    placed,
  };
}

/**
 * The level map for a learner at `position` with no skill scored, as the
 * backend draws it: every skill of a step behind them passed, none ahead, and
 * none shown in a talk or aimed at by one, since no model scores the preview's.
 */
export function levelsAt(position: Position): LevelView[] {
  const current = Math.max(0, levelIndex(position.level));
  const percent = standingAt(position, true).percent;
  return curriculum.levels.map((level, index) => {
    const state = index < current ? "done" : index === current ? "current" : index === current + 1 ? "next" : "locked";
    return {
      number: index + 1,
      name: level.name,
      goal: level.goal,
      state,
      percent: state === "done" ? 100 : state === "current" ? percent : 0,
      steps: level.steps.map((step) => ({
        number: step.number,
        title: step.title,
        focus: step.focus,
        skills: step.skills.map((skill) => {
          const passed = index < current || (index === current && step.number < position.step);
          return {
            label: skill.label,
            text: skill.text,
            passed,
            standing: passed ? ("done" as const) : ("not_started" as const),
            topics: [],
          };
        }),
      })),
    };
  });
}

/**
 * Mirrors `level_ceiling` in progress.rs: the highest level a placement may
 * read off answers this short — under three words on average stays at A0,
 * under five at A1, under seven at A2 — and null when they are long enough.
 */
export function levelCeiling(answers: string[]): string | null {
  if (answers.length === 0) return null;
  const words = answers.reduce((sum, answer) => sum + answer.split(/\s+/).filter(Boolean).length, 0);
  const average = words / answers.length;
  return average < 3 ? "A0" : average < 5 ? "A1" : average < 7 ? "A2" : null;
}

/** `level`, held down to `ceiling` when there is one. */
export function atMost(level: string, ceiling: string | null): string {
  return ceiling && levelIndex(level) > levelIndex(ceiling) ? ceiling : level;
}

/** Each level's colour by its place on the ladder, as Ella Mobile v7 draws
 * them: green, purple, blue, orange, pink, ink. */
export const LEVEL_TONES = ["green", "violet", "blue", "orange", "pink", "ink"] as const;
export type LevelTone = (typeof LEVEL_TONES)[number];

export function levelTone(number: number): LevelTone {
  return LEVEL_TONES[(Math.max(1, number) - 1) % LEVEL_TONES.length];
}
