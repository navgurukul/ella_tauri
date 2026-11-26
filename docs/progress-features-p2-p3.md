# Progress features P2/P3: placement recheck, daily reminders, island map

This follows [progress-features-mobile-to-tauri.md](progress-features-mobile-to-tauri.md)
(P0/P1, merged in #62). Mobile paths are relative to `navgurukul/ella_mobile`,
read at `4f9661f` (main, v1.0.16). Desktop paths are relative to this repo, read
at `94352a6`.

## 1. Plan

### P2 · Placement recheck

**Mobile.**
- `supabase/functions/ella/placement_recheck.ts`: `placementRecheckFor`, `recheckedPosition`.
- `store.ts`: `placementRecheckDue`, `highestStepAt`, and `lessonCadence(…, level)` scoped by level.
- `routes/sessions.ts`: the start gate (~185), the end of a recheck (~723–832), and `PLACEMENT_MIN_TURNS`.
- `routes/learners.ts`: `placement_recheck_due`.
- `lesson.ts`: `completedTargets`.
- `tutor.ts`: the returning-learner placement prompt.
- App: `placement_step.dart` (`_recheckIntro`, retry), `widgets/placement_invitation.dart`, `home_screen.dart`, `app_state.dart` (`offerPlacementRecheck`, `finishPlacementRecheck`).

How it works:
- It is a one-off campaign. Operators set a UTC cutoff in the server's configuration (`config.ts`).
- A learner owes one recheck when they finished a placement with a spoken answer by that cutoff, and no placement has yet recorded a recheck against the same cutoff. A learner who skipped the placement owes none.
- While a recheck is due, every other talk and match is refused (`placement_recheck_required`). Home's Today's talk becomes an invitation: "I’ve updated how I choose your practice. Let’s talk to see what fits your English now." Every other practice button opens the level-check intro.
- The chat is the placement chat, with a returning-learner prompt.
- It is scored only after at least 5 spoken answers (typed answers are left out) and not while Ella is in support. Otherwise it is not complete: "Finish a full talk with Ella so she can check your level." with Try again.
- A scored recheck can move the learner **up or down**:
  - Same level: they keep their step.
  - Another level: they go to the first step at or past the highest step they ever reached there that is not complete, or the last step.
- The completion keeps `placementRecheck {before, previous}`.
- Badges, streaks and skill progress are kept.
- After a recheck that changed levels, lessons count at the level where they were taken, and a stuck-skill lesson already taken at the current level is not offered again.

**Desktop today.**
- Placement only ever moves a learner up (`application.rs` `assess_placement`).
- "Find my level" is offered only while `!standing.placed`.
- There is no due state, no gate, no returning prompt and no downward move.
- `sessions.level_code` already records each talk's level. Each placement's assessment JSON records its standing, and P0/P1 reads `level_ups` from it.

**Missing.** The due rule, the gate, the move in either direction with the resume rule, the record kept with the recheck, the returning prompt, lesson scoping by level, and the Home invitation and level-check intro.

**Proposed Desktop implementation.**
- `src-tauri/src/placement_recheck.rs`:
  - The cutoff is one named constant, `placement_recheck::BRANCH_RECHECK_CUTOFF`. It is not read from the environment or any configuration.
  - `rechecked_position` is ported as a pure function.
- The due rule is a database query over what Desktop already keeps. A placement session with a learner answer, completed by the cutoff, whose assessment read a level (`scored`). No placement assessment may yet carry `recheck.before` equal to the cutoff. **No schema change.**
- A placement started while a recheck is due *is* the recheck. Desktop starts a placement only for a placed learner in that case, so nothing new needs storing on the session.
- `Assessment.recheck` keeps `{before, previous}` and `AppSnapshot.placement_recheck_due` carries the due state. Both are serde-defaulted, so assessments kept before this release still read.
- The gate is in `start_session`, `start_lesson`, `start_word` and `start_chore`. The frontend sends every start to the level check first, as Mobile's `offerPlacementRecheck` does.
- `PlacementBrief`, `NextTalk::Placement` and `placement_opening` gain `returning`. The placement prompt then says they already know each other, as Mobile's `chat_context` does.
- Lesson cadence is scoped to `sessions.level_code` once a recheck has changed levels, with `completedTargets` to match.
- UI:
  - Home shows Mobile's invitation in place of Today's talk.
  - `PlacementTalk` gains a `recheck` mode with an intro, a retry and the level reveal.

**Do not copy.**
- Per-learner server flags.
- The `x-ella-placement-recheck` header and its WeakMap of deps. That exists for Shorebird patches and old phones, and a desktop release ships the app and its rules together.
- The outbox-replay guards.

**Decisions.**
- **Whether Desktop runs a recheck campaign at all, and its cutoff.** This is a product decision, as it is on Mobile, where operators set it. In this branch it is on, with a branch-only cutoff, so the flow can be exercised (section 2).
- Desktop-specific: without a model nothing can read a level. The gate would then lock the learner out of every talk, so a recheck is due only while Ella can judge (`TutorEngine::judges`).

**Tests.**
- Rust:
  - `rechecked_position`: same level, up, down, resuming earned work, last step.
  - Cutoff parsing.
  - The due query: placed before or after the cutoff, a skipped placement, already rechecked.
  - The gate on each start.
  - A recheck moving down and keeping badges' level-ups.
  - A short or typed recheck that is not scored and stays due.
  - No model means not due.
  - Lesson cadence scoped after a level change.
  - The returning prompt.
- Frontend:
  - Home shows the invitation when due.
  - A start while due opens the level check.
  - The `PlacementTalk` recheck intro, retry and result.

### P2 · Daily reminders

**Mobile.**
- `app/lib/reminders/reminder_plan.dart`: the pure plan.
- `local_reminders.dart`: `flutter_local_notifications`, scheduled with the OS.
- `reminders.dart`: the interface.
- `widgets/reminder_sheet.dart` and `reminder_wheel.dart`: Profile's settings.

How it works:
- Reminders are local only, with no push service, and planned again every time the app opens.
- One a day at a chosen time, 19:00 by default, picked from 06:00–23:00 in 5-minute steps, in India time. The first 7 days are daily, then one every 3 days, 7 more times (a month in all). A day already practised is skipped.
- Copy:
  - A lesson-ready message while a lesson waits.
  - The first reminder may mention the streak, for adults only.
  - A 21:30 "last call" for adults (18 or over, with no age counting as under 18) on a day the streak would break.
- Copy never includes the learner's name.

**Desktop today.** Nothing. `Cargo.toml` and `package.json` have no notification plugin, and there is no tray, no login item and no background process.

**Missing.** The plan, the OS delivery and the setting.

**Proposed Desktop implementation.** The plan is a pure function and ports directly. **Delivery is the blocker.**
- `tauri-plugin-notification` on desktop (notify-rust / WinRT toasts) has **no scheduling API**. Its `schedule` option is mobile-only. A reminder could therefore only fire while Ella is running, and a desktop app is rarely open at 19:00.
- Delivering while Ella is closed needs platform code that the OS schedules, with no running app:
  - macOS: `UNCalendarNotificationTrigger` via `UserNotifications`. This needs the signed app bundle and a permission prompt.
  - Windows: `ScheduledToastNotification`. This needs a registered AppUserModelID; the NSIS installer has one.
  - Alternatives are a login item or a tray-resident Ella.

Each option is a product and platform choice with a cost in installer, permissions and support. See section 3.

**Do not copy.** India time (Desktop uses local time), the Android exact-alarm handling and the developer test switch.

### P3 · Island map

**Mobile.** `supabase/functions/ella/islands.ts` (`ISLANDS`, `pathFor`, `crossing`), `schema/0022_islands.sql`, `routes/home.ts` and `routes/sessions.ts` (crossing at the end of a talk), `app/lib/screens/island_home.dart` (1,162 lines), `islands_screen.dart`, `widgets/island/*` (12 files of art), and `screens/recap/island_recap_card.dart`.

How it works:
- Islands are separate from levels.
- Each stop is 2 counted talks plus Ella's lesson.
- Crossing an island needs its stops, words made theirs, and N Word of the day or **News** talks.
- **Umananda, the first island, ends in its own game, Guess it, and only a Guess-it talk crosses it.** Loktak needs News talks.
- A crossing is kept with its stats.
- It is still behind `ELLA_ISLAND_MAP`, and the numbers are marked "first guess".

**Desktop today.**
- Words made theirs (`vocab::word_states`), Word of the day and the lesson cadence exist.
- **News talk and Guess it do not exist.**

**Blocker.** As Mobile defines it, the first island cannot be crossed without Guess it, and a later island needs News. Porting the map without them either strands every learner on the first island or invents new island rules. That needs a product decision; see section 3. Not implemented.

## 2. Implementation status

| # | Feature | Status | Where |
|---|---|---|---|
| 6 | Placement recheck | ✅ Done, **on in this branch** (branch-only cutoff) | See below |
| 7 | Daily reminders | ⏸ Blocked on a platform decision | Section 3 |
| 8 | Island map | ⏸ Blocked on a product decision | Section 3 |

### Placement recheck: what was built

- **`src-tauri/src/placement_recheck.rs`.**
  - The cut-off is `BRANCH_RECHECK_CUTOFF = "2026-11-25T00:00:00Z"`, the day the branch was started. It is the only place the cut-off is set: there is no environment variable, build setting or other configuration for it.
  - **It is a feature-branch value, not a production decision.** Before this merges to production, product decides whether a campaign runs and from when (section 3).
  - `label` keeps the cut-off in the phone's `toISOString` form.
  - `rechecked_position` is Mobile's `recheckedPosition`.
- **Database (`infrastructure/database.rs`).** There is **no schema change**; everything reads columns that already exist.
  - `placement_recheck_due(before)` is owed when:
    - a completed placement with a learner answer read a level (`assessment.scored`) by the cut-off, **and**
    - no placement assessment has `recheck.before` equal to the cut-off yet.
  - `highest_step_at(level)` is the highest step any talk's assessment left them on at that level, or the step a check found them on before moving them.
  - `spoken_answer_ids` lists the answers said aloud.
  - Once a check has moved the learner to another level, the lesson cadence counts talks and lessons at their current level only (`sessions.level_code`). It also returns `completed_targets`, the skills of lessons already taken at that level.
- **Lessons (`lesson.rs`, `progress.rs`).** `progress::stuck_skill_where` is `stuck_skill` with a filter. `lesson::plan` uses it after a level change, so a stuck-skill lesson already taken at the current level, or a skill from another level, is not taught again. This is Mobile's `completedTargets` and `allows`.
- **Service (`application.rs`).**
  - `AppService::recheck_due` is owed only once the cut-off has passed, and only while Ella can judge (see the deviations).
  - `start_session`, `start_lesson`, `start_word` and `start_chore` refuse with "Talk with Ella first so she can check your level." while it is owed.
  - `start_placement` for a placed learner who owes the check opens it as "Talk with Ella", with the returning-learner prompt.
  - `assess_recheck`:
    - It reads only answers said aloud, needs at least `PLACEMENT_MIN_TURNS` (5) of them, and nothing is read once Ella has given her support line (`safety::in_support`).
    - It keeps the existing short-answer ceiling (`level_ceiling`).
    - The learner moves up or down per `rechecked_position`.
    - It writes `Assessment.recheck {before, previous, position}`.
    - A check that could not be read is kept unscored, and the learner still owes the check.
  - `AppSnapshot.placement_recheck_due` is set, and while it is, the talk got ready ahead is the check (`NextTalk::Placement { returning: true }`).
- **Engines.** `placement_system_prompt` has a `returning` variant ("catching up with … You already know each other", Mobile's `chat_context`). `PlacementBrief`, `NextTalk::Placement`, `placement_opening` and `placement_readiness` carry it, so the cached prompt prefix stays the chat's own.
- **Window.**
  - Home shows Mobile's invitation in Today's talk's place.
  - Every other start (topics, lesson, Word of the day, partners, badges, the level map) opens the check's invitation instead (`App.tsx` `checkFirst`, Mobile's `offerPlacementRecheck`).
  - `PlacementTalk` has a `recheck` mode:
    - Intro: "Start talking" or "Back to Home".
    - The chat itself.
    - The level reveal.
    - Or "Finish a full talk with Ella so she can check your level." with "Try again".
  - A check skipped part way is still read, as on Mobile.
  - The preview bridge reports `placement_recheck_due: false`, as the backend does without a model.

With this branch's cutoff, every learner whose placement read a level before 25 November 2026 is asked for the check the next time they open Ella, provided a model can judge. That includes a laptop's existing learner.

### Where Desktop deliberately differs from Mobile

- **A cut-off constant, not a server setting.** Desktop has no server, so the cut-off is fixed in a release. Mobile's `x-ella-placement-recheck` header, its WeakMap of deps and its outbox-replay guards exist for Shorebird patches and phones on older builds, so they were not ported.
- **Not owed without a model.** Mobile's server always has one. On Desktop, demo mode, or a laptop whose model failed with no cloud, could never read a level, so the gate would lock the learner out of every talk. While Ella cannot judge (`TutorEngine::judges`), no check is owed.
- **"Placed by the cut-off" means a placement that read a level.** On Mobile every placement with a learner turn places. On Desktop, a placement with fewer than 5 answers places nobody, so it does not count either.
- **A recheck is recognised, not stored.** Mobile marks the session (`modeData`). On Desktop, a placement started by a placed learner while a check is owed *is* the check. Desktop only offers a placed learner the placement then, and this avoids a new column.
- **Same opening line.** Mobile changes its greeting from "Hi X, I am Ella!" to "Welcome back, X!". Desktop's opening, "So X, tell me about your day so far!", greets nobody as new, so it is unchanged. Only the system prompt changes.
- **The check's name** on the talk screen and in the session list is "Talk with Ella", Mobile's title for it.
- **A check whose cut-off was answered while it was being read** keeps the learner where they are, as Mobile's `eligible` guard does. A check started while one was owed but assessed after none is owed is assessed as an ordinary placement, which only moves a learner up. This needs a second check to finish in between, which the gate makes very unlikely on a single-learner laptop.

## 3. Product and platform decisions needed

1. **Placement recheck: whether and when.** Should a Desktop release run a level-check campaign, and with what cut-off? Mobile's reason ("I’ve updated how I choose your practice") is about Mobile's placement changes, and Desktop's placement may not have changed the same way. The invitation copy is Mobile's verbatim; confirm it fits Desktop. In this branch the check is on with a branch-only cutoff (`BRANCH_RECHECK_CUTOFF`), which must be decided, or the campaign removed, before production.
2. **Daily reminders: delivery.** Choose one:
   - (a) **In-app only.** Add `tauri-plugin-notification`, and notify only while Ella is open. Low cost, little value at 19:00.
   - (b) **OS-scheduled.** macOS `UNCalendarNotificationTrigger` and Windows `ScheduledToastNotification`, through platform code. They fire with Ella closed. This needs a notification-permission prompt, signing and an AUMID on Windows, plus per-OS code and tests.
   - (c) **Resident.** A login item or tray icon so Ella runs in the background. Heavier, and it changes the app's footprint.

   Mobile's rules port either way: the 19:00 default, a week daily then every 3 days, skipped on days practised, lesson-ready copy, and a streak last call for adults only. Also decide where the setting lives (Profile → Settings, as Mobile) and whether to use the local clock (recommended; Mobile uses India time).
3. **Island map: scope.** As Mobile defines it, the first island (Umananda) is crossed only by a Guess-it talk, and Loktak needs News talks. Desktop has neither mode. Options:
   - (a) Port News and Guess it first.
   - (b) Ship Desktop islands with goals limited to words and Word of the day. That is new behaviour, needs product sign-off, and diverges from Mobile.
   - (c) Wait until Mobile takes it out from behind `ELLA_ISLAND_MAP` and fixes its "first guess" numbers.

   It is also art-heavy (12 widget files, 1,100+ lines of island Home on Mobile) and needs a small `islands` table for entering and crossing, which are events.

## 4. Remaining work

- The decisions in section 3, then reminders and islands as decided.
- Validate the level check in a real build with a model and a cut-off. The automated tests cover the flow end to end against a scripted model; see the final report.
