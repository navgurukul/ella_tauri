/**
 * Calendar days as the learner lives them. Shared by the streak on screen and
 * the browser preview's progress, so both put a talk on the same day the
 * backend does (`date(created_at, 'localtime')`).
 */

/** The learner's own calendar day, not UTC's: a talk at half past midnight in
 * Pune belongs to that day, not the one before. */
export function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** The same time of day, some whole calendar days away. Steps by calendar day
 * rather than by 24 hours, so a clock change does not skip or repeat a day. */
export function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}
