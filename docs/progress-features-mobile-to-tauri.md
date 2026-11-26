# Progress features: Ella Mobile → Ella Desktop

Investigation only; no code was changed. Mobile paths are relative to the
`navgurukul/ella_mobile` repo, Desktop paths to this repo. Mobile was read at
`08407be`, Desktop at `8878eac`.

## 0. How the two apps differ

| | Ella Mobile | Ella Desktop |
|---|---|---|
| Where the progress rules run | The server: the Supabase Edge Function `ella` (Deno/TS, `supabase/functions/ella/`) | Rust inside the app (`src-tauri/src/`). The frontend also derives the streak and the badges (`src/lib/presentation.ts`) |
| Where the data lives | Postgres schema `ella`, many learners (`supabase/schema/0001…0023_*.sql`) | Local SQLite, one learner per install (`src-tauri/src/infrastructure/database.rs`) |
| How the UI gets it | REST: `GET /v1/learners/me`, `/v1/home`, `/v1/levels`, `/v1/learners/me/words`, `POST /v1/sessions/:id/end` (`app/lib/api/api_client.dart`, `app/lib/api/models.dart`) | Tauri IPC: `bootstrap` → `AppSnapshot`, `assess_session` → `Assessment`, `complete_session` → `SessionSummary`, `levels` → `LevelView[]` (`src-tauri/src/ipc.rs`, `src/lib/bridge.ts`, `src/types.ts`) |
| Practice calendar | India time (`Asia/Kolkata`) | The laptop's local time (`src/lib/days.ts`, `date(created_at,'localtime')`) |
| Rollout | Per-learner server flags (`ELLA_XP`, `ELLA_ISLAND_MAP`) and a server-set placement-recheck cutoff (`supabase/functions/ella/config.ts`) | No flags. A release ships to everyone |
| How answer time is measured | `turns.listening_ms`: how long the mic was open, as the app measured it | `messages.spoken_ms` (`telemetry.rs` `spoken_ms()` → `audio_after_vad_ms`). For a streamed answer (the native speech engine, the main route) this is the whole recording, which is mic-open time as on Mobile. For a buffered answer it is the recording with leading and trailing silence trimmed. `None` for typed answers. *(Corrected in review: the investigation first said "the speech the VAD kept" for every answer.)* |

Desktop already says it ports Mobile's rules: the header of `src-tauri/src/progress.rs` reads "The same rules as Ella Mobile". So the core engine is shared, and the gaps are features built on top of it.

---

## A. Progress features found in `ella_mobile`

| # | Feature | Where implemented | How it works | What the learner sees |
|---|---|---|---|---|
| 1 | **Adaptive mastery and target selection** | `supabase/functions/ella/progress.ts`: `bucket`, `passed`, `towardsPassing`, `levelPercent`, `candidates`, `priority`, `chooseTarget`, `stuckSkill`, `applyScores`. Stored in the `skill_progress` rows read by `store.ts` `skillProgress` | A judged skill counts only at ≥0.70 confidence. Mastery rises by `0.4·c·(1−m)`. A skill is owned at ≥0.75 once shown in 2 talks on 2 topics, and emerging from 0.25. Grammar passes at emerging; the other strands must be owned. Priority weights: need 0.35, uncertainty 0.20, review due 0.20, order 0.15, variety 0.10, recency −0.25. A skill missed twice running is stuck | Indirect: which skill each talk aims at, the level %, the skill states |
| 2 | **Step and level advancement** | `routes/sessions.ts` ~1484–1522 (`advanced: "step" \| "level"`, `store.advance`). `badges.ts` `completedLevels` | When the current step completes, the learner moves to the next step or level, and the session's `completion` JSON records it (`schema/0007_activity_stats.sql`) | `summary.advanced` drives the celebration (#6) |
| 3 | **Level card and level track (Profile)** | `app/lib/screens/profile_screen.dart` 615–897. Payload built in `routes/learners.ts` `learnerView` → `progress{level_number, level_count, earned_from_level, steps, completed_skills, total_skills, level_percent, skills[]}` | `levelPercent` = (steps behind + the share of this step's skills passed) / steps. `earnedFromIndex` (`routes/levels.ts`) keeps levels below the start hollow until placement | "MY LEVEL" card: number, name, "NN% to level N+1", a track of stops |
| 4 | **Level map ("Levels and badges") and the level page** | `app/lib/screens/levels_screen.dart` (Ladder 25–460, `LevelScreen` 479+, `_EllasSkill` 1492, skill rows 1815). Server: `routes/levels.ts` `levelsView`, `ellasSkill` | Level states: done / open / current / next / locked. Each skill has `passed`, **`progress` (`towardsPassing`, capped at `NOT_YET` = 0.9)**, `topics[]`, `aimed`, **`focus`** (the skill Ella is teaching now: the lesson's target, else `aimFor`) | Path of levels; per-level page with a step bar, a lilac card for Ella's current skill, two-circle skill rows, "Shown in …", "Use it again in a talk on a new topic", and the labels Done / **Almost there** / Practising / Not started (`models.dart` ~440) |
| 5 | **"Skills grew" recap card** | `recap.ts` (`RECAP_MAX_SKILLS` = 3, `SkillView{label,count}`). App: `screens/recap/recap_cards.dart` ~1649, `state/recap_data.dart` 349–376 | The skills whose sightings went up, the talk's aim first | Up to 3 columns of 1–5 blocks: green "Today" on top, purple for earlier talks |
| 6 | **Step-up / level-up celebration** | `app/lib/widgets/moved_on_sheet.dart`. Triggered at `recap_screen.dart` 134–137; `_celebrateLate` in `app_state.dart` ~965 if the learner left before the server answered | Reads `summary.advanced`, `level_name`, `step`, and the ladder fetched while the recap plays to name the step and its skills | "Step N done!" with the step title and green skill chips, or "Level up! You've reached X". A 5-pill step track animates, then "STEP n · YOU'RE HERE"; Ella hops; confetti (skipped under reduce-motion) |
| 7 | **Streak and week strip** | `store.ts` `streak()`, `streakWeek()` (~2445–2480) over the `talk_days` table (rebuilt from learner turns in `schema/0007`). App: `home_screen.dart` 224–312, `profile_screen.dart` 928, recap streak card `recap_cards.dart` ~1931 plus `RecapStreakData.of` in `recap_data.dart` 381–476 | Consecutive days ending today, or ending yesterday if today has no talk yet. The week runs Monday to Sunday with marks done/today/future. The recap gets `{days, counted_before, week[]}` | Home flame pill. Recap card: the number rolls up, today's dot pops in, and a chip reads "New streak", "Done for today" or "<Mode> counts". **Shown once per day per phone** (`streakSeenToday` / `markStreakSeen`, `app_state.dart` 1877–1895). "TOMORROW · Day N+1" |
| 8 | **XP** | `supabase/functions/ella/xp.ts` (`MS_PER_XP` = 10 000, `MOST_MS_PER_WORD` = 3 000, `earnedMs`, `xpToSend`). Kept in `learners.xp`, `xp_carry_ms`, `xp_day`, `xp_today` and `turns.xp_ms` (`schema/0023_kept_xp.sql`, `store.ts` `#awardXp`). App: `app/lib/widgets/xp.dart` (`XpPill` :42, `XpSheet` :87) | 1 XP per 10 s of voiced answer time, capped at 3 s per word heard. Typed, unmeasured, rude and "misheard" answers earn 0. Kept rather than derived, so a rule change only affects later answers. Flagged per learner by `ELLA_XP` | Home pill (spark + total). The sheet shows "YOUR XP", "+N today", how XP is earned, and "Typed answers don't count". Hidden when `xp` is absent. Not shown in the recap |
| 9 | **Milestone badges** | `supabase/functions/ella/badges.ts`: `milestoneBadges`, `practiceRewards`, over `MilestoneHistory`. App: `widgets/badges.dart` (`openBadges` ranking :134, `practiceBadgeMessage` :33, sheet 418–689), `widgets/badge_art.dart` | Badge ids: `hello`, `first`, `s3`/`s7`/`s30`, `t50`, **`level-1…6`**. Earned dates come from committed history, so an earned badge never expires. `practiceRewards` claims each badge once, saved with the talk's completion | Profile badge row with "N of M" and "NEXT UP" (up to 3). Badge sheet with a status pill and "WHERE TO EARN IT". **Recap card "BADGE EARNED"** (`RecapPracticeBadgeCard`, `recap_cards.dart` 387; inserted at `recap_engine.dart` 380) |
| 10 | **Talk-partner scene badges** | `store.ts` `badges()` → `BadgeRecord{count, tries, earnedOn}`. `routes/learners.ts` maps `PARTNERS × scenarios`. Recap `role_play{…badge}` (`recap_engine.dart` 675–735) | A badge is earned when a scene's goal is met, and counted each time | "Goal met! NEW BADGE" or "EARNED AGAIN ×N", or "So close!" with Try again |
| 11 | **Activity stats** | `store.ts` `activity()` → `Activity{talks_done, different_words, listening_seconds, timed_turns, untimed_turns, days_practised}`. App: `profile_screen.dart` 902–955 | Totals over the learner's turns | Profile: DAY STREAK, TALKS DONE, VOICE TIME ("—" when only unmeasured turns exist), "Talking since <joined>" |
| 12 | **Words made theirs ("My words")** | `supabase/functions/ella/vocab.ts` (`FIRST_LOOK_TALKS` = 3, `YOURS_TALKS` = 2, `RECAP_YOURS` = 4, `RECAP_ALMOST` = 3, `CLOUD_WORDS` = 40, `LITTLE_WORDS`, `wordStates`, `voiceOf`), `english_words.ts`, `profanity_words.ts`. Endpoint `GET /v1/learners/me/words`. App: `screens/words_screen.dart`, `widgets/learner_words.dart` (`VoiceCard` :142), `widgets/word_cloud.dart`, `profile_screen.dart` 994–1270 | Only voiced answers count. A word counts only if it is in the English list, is not a little word, a swear word or the learner's name, and is not an echo of Ella's previous line. Forms of a word are grouped. A word is "almost" theirs after 1 talk, and "yours" after 2 talks on 2 topics. Worked out each time, never stored | Profile "My words" card. Word cloud. "How you speak" card (speaker type, when they talk most, pace). Recap "New words" card with YOURS NOW / ALMOST YOURS. A pink glow on new words until My words is opened |
| 13 | **Lesson cadence, countdown, nudge and mentor** | `supabase/functions/ella/lesson.ts` (`LESSON_AFTER` = 2, `lessonState` → `{plan, talksLeft}`). `schema/0010_lesson_cadence.sql` (`sessions.counted`, `lesson_ready`, `lesson_strengths`). App: `home_screen.dart` 419–546, `widgets/mentor_sheet.dart`, `widgets/lesson_nudge.dart` 228, `partners_screen.dart` `_MentorCard` :129 | A lesson is ready after 2 counted talks or when a skill is stuck, and stays ready until taken. "Overdue" means a counted talk went past a ready lesson | Home card: **"N more talks to your lesson with Ella"** (`home.talks_to_lesson`), or a lilac "LESSON WITH ELLA". The MentorSheet shows once per new lesson. The nudge sheet appears before other talks when the lesson is overdue |
| 14 | **Placement and placement recheck** | `supabase/functions/ella/placement_recheck.ts` (`placementRecheckFor`, `recheckedPosition`). App: `screens/onboarding/placement_step.dart` 92, 262–330; `app_state.dart` 535, 1063 | After a server-set date, placed learners retake the placement. The result can move them **up or down**, and they resume at the first step at or past the highest they reached there that is not yet complete. Feeds `level-N` badges | "Level check" intro, then a retry if the talk was too short, then "YOUR LEVEL" plus any badges earned |
| 15 | **Island map** | `supabase/functions/ella/islands.ts` (`ISLANDS`, `pathFor`, `crossing`, `PathView`, `Crossing`). `schema/0022_islands.sql`. App: `screens/island_home.dart`, `screens/islands_screen.dart`, `widgets/island/*`, `screens/recap/island_recap_card.dart` | Each stop is 2 counted talks plus a lesson. Crossing an island needs its stops and its goals: a word count, N Word-of-the-day or News talks, and sometimes the Guess-it game. Crossings are kept with stats. Flagged by `ELLA_ISLAND_MAP`; the numbers are marked "first guess" | Island Home bar ("N more talks / then Ella's lesson"), a map, a goals sheet, an islands list with stamps, and a recap "Island crossed!" card with an arrival animation |
| 16 | **Daily reminders** | `app/lib/reminders/reminder_plan.dart`, `local_reminders.dart`, `widgets/reminder_sheet.dart` (`flutter_local_notifications`) | One reminder a day, at 19:00 India time by default, planned 5 days ahead, skipped on days already practised. The copy changes when a lesson is ready or the streak is at risk. A 21:30 "last call" goes to **adults only**; under-18s never get streak-loss messages | OS notifications. Profile → "Daily reminder" |
| 17 | **Recap card sequence** | `app/lib/state/recap_engine.dart` 315–388; hold times in `theme.dart` ~1324 | Card order: hero → mode → went well → words → fix → skills → offline → practice badges → streak → island. Auto-advances with a progress bar, pauses on hold, and does not auto-advance under reduce-motion or a screen reader. A talk is "short" below 3 turns or 12 words | Stories-style recap |

---

## B. `ella_mobile` → `ella_tauri` mapping

Status: ✅ at parity · 🟡 partial · ❌ absent.

| Mobile feature | Desktop equivalent | Status | Gap | Recommended approach |
|---|---|---|---|---|
| 1. Mastery and targets | `src-tauri/src/progress.rs` (`bucket`, `passed`, `level_percent`, `candidates`, `priority`, `choose_target`, `stuck_skill`, `apply_scores`, `grown_skills`); `skill_mastery` table (`database.rs` ~1367) | ✅ | None: the constants and formulas match | Keep in sync. Add a shared fixture test if the rules drift |
| 2. Step and level advancement | `application.rs` `assess_talk` (~2488–2510) sets `Assessment.advanced` | ✅ | — | — |
| 3. Level card and track | `src/components/LevelCard.tsx` (`LevelCard`, `LevelTrackCard`), `ProfileScreen.tsx` | ✅ | Desktop has no `earned_from_level` "hollow levels" distinction. Its placement only moves up, so this matters little | Optional. Only needed if placement recheck (row 14) is added |
| 4. Level map and page | `src/components/LevelsScreen.tsx`; `domain.rs` `LevelSkillView{label,text,passed,standing,topics}`; `progress::standing` | 🟡 | (a) No **`towardsPassing`** fraction, so there are no partial bars. (b) No **"Almost there"** standing. (c) No per-skill **`focus`**, so there is no "Ella is teaching this" card on the level page | **Direct port, small.** Add `towards_passing(skill, p)` and `NOT_YET` to `progress.rs`. Add `progress: f64` and `focus: bool` to `LevelSkillView` (filled where `standing` is filled, `application.rs` ~2597–2637; the focus comes from `lesson::plan(..).target` or `choose_target`). Mirror the fields in `src/types.ts` and render them in `LevelsScreen.tsx` |
| 5. Skills-grew card | `progress::grown_skills`; `SummaryScreen.tsx` "Skills grew" tile | ✅ | — | — |
| 6. Moved-on celebration | `SummaryScreen.tsx` `MovedOn` (~319–338): "Step complete!" / "Level up!", called once via `onCelebrated` | 🟡 | No step title, no skill chips from the finished step, no step-track animation, and no confetti. If the learner leaves before assessment finishes, the celebration is lost (no `_celebrateLate`) | **Adapt the UI.** The data is already local: `LevelView[]` from `levels` plus `Assessment.advanced`. Add a celebration sheet in React. Persist "celebrated" on the session (e.g. a column next to `sessions.assessment`) so a late assessment still celebrates on Home |
| 7. Streak and week strip | `src/lib/presentation.ts` `streak()` (~152–181), `streakRecap()` (~616–635); `HomeScreen.tsx` 151–177; `Sidebar.tsx`; `SummaryScreen.tsx` streak tile | ✅ / 🟡 | Same rules. The recap streak tile shows on every recap; Mobile shows it once a day | Optional: once a day, using `localStorage` keyed by `dayKey()`. **Do not** copy the India-time calendar: Desktop's local time is right |
| 8. XP | None | ❌ | No XP rule, store or UI | **Rust implementation, small to medium.** See C.1. Data is already captured: `messages.spoken_ms` per voiced answer, `NULL` for typed answers. Derive XP in `Database::progress()` (`database.rs` 629–738) as `Σ min(spoken_ms, 3000 × words(content))` / 10 000 overall and for today. Add `xp_total` and `xp_today` to `LearnerProgress` (`domain.rs` ~75–104, `types.ts`). Add `XpPill` / `XpSheet` beside the streak on Home and in the Sidebar |
| 9. Milestone badges | `presentation.ts` `BADGES` (~338–411): `hello`, `first-talk`, `streak-3/7/30`, `talks-50`, derived from `progress.talks` and `progress.days` | 🟡 | **No `level-1…6` badges.** No recap "BADGE EARNED" card for milestone badges (only chores show "New badge") | **Port to the frontend, small.** Add level badges to `BADGES`, earned from `standing` (level index) plus the day of the talk that advanced (`sessions.assessment.advanced == "level"`). That needs a `level_ups: [{level, day}]` list in `LearnerProgress`, read from sessions' assessment JSON. For the recap card, compare `badgesOf(progressBefore)` with `badgesOf(progressAfter)` the same way `streakRecap` does, and show newly earned ones. No stored claims are needed, because everything is derived locally and atomically |
| 10. Scene badges | `presentation.ts` `bargainer`, `deposit` from `ledger_state` `goal_met`; `Badges.tsx`; `SummaryScreen.tsx` 283–315 | ✅ (fewer scenes) | Desktop ships fewer partner scenes, by design (comment at `presentation.ts` ~328–334) | Add a badge with each new chore. Nothing to port wholesale |
| 11. Activity stats | `LearnerProgress{talks_finished, answers, spoken_ms, spoken_answers, days}`; `ProfileScreen.tsx` 179–190; `HomeScreen.tsx` "This week" 180–216 | ✅ | Desktop shows less: no "days practised" or "Talking since". Desktop has a weekly digest that Mobile lacks | Optional: show `days.length` as days practised. Trivial |
| 12. Words made theirs | None. Word of the day exists (`src-tauri/src/words.rs`, `WordOfTheDay.tsx`) but there is no per-learner vocabulary | ❌ | Needs an English word list, little-word and profanity lists, form grouping, the echo rule, the almost/yours states, and UI | **Rust port, medium to large.** New `src-tauri/src/vocab.rs` ported from `vocab.ts`, computed from `messages` (`speaker='learner'`, `spoken_ms IS NOT NULL`, the previous Ella message for the echo rule). Word lists as JSON in `shared/`. The `english_words.ts` list is large, so check bundle size. UI: a Profile card, plus a recap "New words" tile later. Skip the word cloud and `voiceOf` at first |
| 13. Lesson cadence and countdown | `src-tauri/src/lesson.rs` (`LESSON_AFTER` = 2, `LessonCadence{talks, overdue, held, …}`, `plan`); `HomeScreen.tsx` 58–98; `LessonNudge.tsx`; `CastScreen.tsx` mentor card | 🟡 | **No "N more talks to your lesson with Ella" countdown.** `cadence.talks` is computed but not sent. No one-time "I have a lesson for you" sheet | **Small.** Add `talks_to_lesson: Option<u32>` (`LESSON_AFTER − cadence.talks` when no plan) to `AppSnapshot` (`application.rs` ~313; `types.ts` `AppSnapshot`). Show it on Home's Today's-talk card and the CastScreen mentor card |
| 14. Placement recheck | Placement only: `application.rs` ~444, 2296–2301 (only moves up), `progress::placement_ends`, `level_ceiling`; "Find my level" while `!standing.placed` | ❌ | No recheck, and no downward move | **Defer.** On Mobile this is a one-off server campaign, with its cutoff set in the server's configuration. Desktop would need its own trigger, e.g. "Check my level again" on the Levels screen, plus `recheckedPosition`'s resume rule. Needs a product decision, see D |
| 15. Island map | None. There is no News talk or Guess-it game on Desktop either | ❌ | The whole feature, plus the modes it depends on (News, Guess) and words made theirs (#12) | **Do not port yet.** It is experimental and flagged on Mobile, its numbers are "first guesses", and it depends on #12 and on modes Desktop lacks. Revisit once Mobile settles it |
| 16. Daily reminders | None. No notification plugin in `src-tauri/Cargo.toml` or `package.json` | ❌ | Desktop apps are not running in the background the way a phone schedules notifications | **Nice-to-have; needs a platform decision.** `tauri-plugin-notification` can only notify while Ella runs, and the OS can't schedule them ahead the way `flutter_local_notifications` does. Keep the **under-18 rule** if built |
| 17. Recap sequence | `SummaryScreen.tsx`: a single page of tiles, not timed cards | 🟡 (different by design) | — | **Do not copy** the Stories-style auto-advancing cards. A desktop window suits the one-page tile layout. Only add the missing tiles (badge earned, new words) |

### Unused or half-used Desktop fields found on the way
- `SessionSummary.headline` and `encouragement` are produced in Rust but never rendered in `.tsx` (only in tests).
- The `chore_progress(chore_id, attempts, passed_at, best)` table is written but no progress display reads it.

---

## C. Recommended implementation plan

### 1. Highest priority (high value, small effort, data already local)
1. **XP.** It is the one headline progress feature Mobile has and Desktop lacks, and `messages.spoken_ms` already holds the input.
   - Rust: a pure `xp` module, `src-tauri/src/xp.rs`, with `MS_PER_XP` = 10 000, `MOST_MS_PER_WORD` = 3 000, and `earned_ms(spoken_ms, content)`.
   - Aggregate in `Database::progress()`.
   - Add `LearnerProgress.xp_total` and `xp_today`.
   - UI: `XpPill` and `XpSheet` in `src/components/`, mounted in `HomeScreen.tsx` and `Sidebar.tsx`.
   - Derive XP rather than keeping it. Because Desktop is local and single-learner, nothing needs to be kept unless the rule changes later; if it does, add a column then.
2. **Lesson countdown** (`talks_to_lesson` on `AppSnapshot`). Roughly a ten-line change in Rust plus copy on Home and CastScreen.
3. **Level-map partial bars, "Almost there" and Ella's focus skill** (`progress::towards_passing`, `LevelSkillView.progress` and `focus`, `LevelsScreen.tsx`).

### 2. Medium priority
4. **Level-complete badges** (`level-1…6`) plus a **"Badge earned" recap tile**. Both are frontend-derived in `presentation.ts`; the only Rust addition is the list of level-up days in `LearnerProgress`.
5. **Richer moved-on celebration**: step title, the finished step's skill chips, the step track, and a late celebration on Home. Needs a persisted "celebrated" flag.
6. Small parity touches: streak tile once a day, "days practised" and "Talking since" on Profile, and rendering `SessionSummary.encouragement`, or deleting it.

### 3. Nice to have
7. **Words made theirs**: `vocab.rs`, word lists in `shared/`, a Profile card, then a recap tile. Most valuable of this group, but the largest effort.
8. **Placement recheck**, as a learner-initiated "Check my level again" that can move down and resumes per `recheckedPosition`.
9. **Reminders** via `tauri-plugin-notification`, in-app only.
10. **Island map**, only after #7 and only if Mobile keeps it.

### 4. Dependencies and blockers
- The island map depends on words made theirs, the News talk and the Guess-it game, none of which Desktop has.
- Level badges are best done after the level-up history is exposed (C.4). Placement recheck would make their earned dates more involved.
- XP's time measure: on the main (streamed) route Desktop's `spoken_ms` is mic-open time, as on Mobile. Only buffered answers are trimmed of leading and trailing silence, and earn slightly less. *(Corrected in review.)*
- Words made theirs needs `english_words.ts` (and `profanity_words.ts`) converted to JSON. Check the installer size and the licence or origin of the list.
- Every new `LearnerProgress` / `AppSnapshot` field must change in three places: `domain.rs`, `src/types.ts`, and the test fixtures in `src/App.test.tsx` and `src/lib/presentation.test.ts`.

### 5. Suggested sequence
1. `talks_to_lesson` (warm-up; touches the snapshot pipeline end to end)
2. XP (Rust module + DB aggregate + pill/sheet + tests)
3. `towards_passing` / `focus` on the level map
4. Level badges + recap "Badge earned" tile
5. Moved-on celebration upgrade
6. Words made theirs
7. Then decide on placement recheck, reminders and islands

---

## D. What not to copy, and open questions

**Do not copy:**
- Per-learner server rollout flags (`ELLA_XP`, `ELLA_ISLAND_MAP`, …). Desktop ships to everyone in a release; use a build-time or settings flag if a trial is wanted.
- India-time practice days. Desktop correctly uses local time.
- Mobile's kept-XP columns and their catch-up migration (`schema/0023_kept_xp.sql`, `#awardXp`). They exist because of multi-version servers and rollbacks; Desktop can derive XP.
- Idempotent `request_id` turn handling, accounts, and the Supabase auth around progress.
- The Stories-style timed recap engine (`recap_engine.dart`).
- The `practiceRewards` "claim once" bookkeeping. Desktop derives badges from history in one process, so before/after comparison is enough.

**Open questions for the team:**
1. Should XP be shown to everyone at once, or behind a setting while it is tried out, as Mobile does?
2. `spoken_ms` is mic-open time on the main route and edge-trimmed on the buffered fallback. Is that small difference acceptable, or should the fallback also record mic-open time?
3. Placement recheck on Desktop: learner-initiated only? May it move a learner down?
4. Is the island map a direction Desktop is meant to follow, or is it Mobile-only?
5. Reminders: is an in-app-only reminder worth building on a desktop?

---

## E. Implementation status

Branch `feature/progress-features-mobile-to-tauri`. P0 and P1 are implemented; P2 and P3 are not, by decision.

| # | Feature | Status | Where |
|---|---|---|---|
| 1 | Lesson countdown | ✅ Done | `lesson::talks_to_lesson` shares one rule (`talks_left`) with `lesson::plan`. `AppSnapshot.talks_to_lesson` carries it. Shown on Home's Today's talk and the mentor card in `CastScreen.tsx` |
| 2 | Level-map progress | ✅ Done | `progress::towards_passing` / `NOT_YET`. `LevelSkillView.towards_passing` (percent) and `ellas_focus` (the lesson's target, else `choose_target`). Bars and an "Ella's focus" card in `LevelsScreen.tsx`. "Almost there" already existed (`SkillStanding::Shown`) |
| 3 | XP | ✅ Done | `src-tauri/src/xp.rs` (`earned_ms`, `tally`, `MS_PER_XP`, `MOST_MS_PER_WORD`), worked out in `Database::progress` into `LearnerProgress.xp`. `XpPill` / `XpSheet` (`src/components/Xp.tsx`) in the sidebar. No new storage |
| 4 | Level badges and recap tile | ✅ Done | `LearnerProgress.level_ups` is read off each talk's kept assessment. `level-1…6` rules in `presentation.ts` (`levelPassed`). `newlyEarned` drives the recap's "Badge earned" tile. No "award once" records |
| 5 | My Words | ✅ Done | `src-tauri/src/vocab.rs` (Mobile's `vocab.ts`), `shared/english_words.txt` (ESDB, with its notice), `Database::spoken_turns`. `AppSnapshot.words` feeds Profile's "My words" card. `SessionSummary.new_words` feeds the recap's "New words" tile |
| 6 | Placement recheck | ✅ Done in P2/P3, on in that branch with a branch-only cutoff | [progress-features-p2-p3.md](progress-features-p2-p3.md) |
| 7 | Daily reminders | ⏸ Blocked on a platform decision | [progress-features-p2-p3.md](progress-features-p2-p3.md) |
| 8 | Island map | ⏸ Blocked on a product decision | [progress-features-p2-p3.md](progress-features-p2-p3.md) |

### Where the implementation deliberately differs from Mobile
- **XP time.** Desktop pays `messages.spoken_ms`. With the native speech engine, answers are streamed and this is the whole recording, which is mic-open time, the same measure as Mobile's `turns.listening_ms`. On the buffered fallback, leading and trailing silence is trimmed (`audio::trim_to_speech`, keeping 120 ms before and 180 ms after), so those answers earn slightly less. The rule is Mobile's (10 s per XP, at most 3 s per word heard), and the per-word cap is what stops a mic left open in silence from paying. `xp::MS_PER_XP` is the one number to tune, and the window reads it from the backend. *(Corrected in review: this section first said every answer was VAD-trimmed.)*
- **XP is derived, not kept.** Changing the rule changes past answers' XP too. Mobile keeps XP so that rule changes only apply forward.
- **XP for flagged answers.** Mobile pays nothing for an answer its safety screen flagged, or one Ella asked to hear again. Desktop keeps no mark with an answer: a flagged answer is stored censored, with its spoken time, so it still earns. Excluding it would need a flag column on `messages`.
- **XP placement.** The pill sits in the sidebar above the streak pill; Mobile puts it in Home's header. Desktop's streak lives in the sidebar, so the two stay together.
- **Level badges with no day.** A level passed before assessments recorded the move (e.g. a placement saved as `null` by an older release) is earned with no date, as Mobile does for legacy progress.
- **The recap's badge tile** leaves out level badges ("Level up!" shows those) and scene badges (the role-play band shows those), as Mobile's recap does.
- **My Words swear filter.** Swear words are those `safety` (rustrict) already refuses, rather than Mobile's dsojevic list.
- **My Words ordering.** A word's "first said" is its answer's place in the history, ordered by when each talk started, where Mobile uses the answer's timestamp. They differ only when an older talk is resumed after a newer one, and then only in tie-breaks.
- **My Words false positives from rustrict.** Its "inappropriate" set is broader than Mobile's list: words like "breast" (chicken breast), "bloody", "hell", "naked" never count on Desktop.
- **My Words scope.** Desktop shows counts plus the three words said most. Mobile's word cloud, "How you speak" card (`voiceOf`) and the "new words" glow are not ported.
- **My Words in the recap.** The tile appears only when the talk made something new; Mobile sends an empty card in that case.
- **The level map's "Done" for unplaced learners.** Desktop's level map marks levels under an unplaced learner's start level as done. The badges follow Mobile's stricter rule (not earned without a placement). The map was left unchanged; see follow-ups.

### Fixed in review
An engineering review of the branch found these, all fixed with tests:
- **My Words used the learner's full name.** The own-name rule compared tokens with "Sunny Kapoor", so "sunny" could count as a word. It now uses the first name, as Mobile's `firstName` does. It uses Desktop's existing `first_name`, the first word. Mobile's also skips titles such as "Mr." and splits on `_` and `.`; Desktop's greeting does neither, and My Words now agrees with it.
- **The lesson countdown could promise a lesson that never came.** While Ella can't teach (no model and no cloud), Home offers no lesson (`lesson_now`), but the countdown still counted down. It is now hidden whenever `teaches_lessons()` is false.
- **A placement chat's summary carried a "New words" tile.** Mobile gives a placement no words card; neither does Desktop now.
- **My Words' "after 3 talks" gate didn't count Word of the day talks.** Mobile's `talksDone` does. It is now `Database::talks_done`, a copy of Mobile's query (completed, spoken in, placement aside).
- **The swear check ran rustrict for every word of every answer on every view.** Each word is now checked once and remembered. Over 2,000 answers a view went from about 80 ms to about 5 ms (release build).
- **The XP query failed on an unreadable timestamp.** It now treats such an answer as not today, instead of failing Home.
- **The XP documentation misdescribed `spoken_ms`.** Corrected above and in `xp.rs`.

### P2: product decisions needed

**6. Placement recheck.** On Mobile this is a one-off server campaign: after a cutoff set in the server's configuration, placed learners retake the placement, which can move them **down** as well as up. `recheckedPosition` then resumes them at the first step at or past the highest they reached on that level that is not yet complete. Desktop's placement only ever moves a learner up (`application.rs`, placement assessment) and is offered only while `!standing.placed`. Decisions needed:
1. Is a recheck learner-initiated ("Check my level again" on the Levels screen), prompted by the app (e.g. after N talks or a long gap), or both?
2. May a recheck move a learner **down**? If so, what happens to level badges already earned? Mobile keeps them, since they are read from history.
3. Should Desktop port `recheckedPosition`'s resume rule, which needs the highest step reached per level? Desktop does not keep that today.

**7. Daily reminders.** Mobile schedules OS notifications ahead (`flutter_local_notifications`, `app/lib/reminders/reminder_plan.dart`): one a day, 19:00 by default, skipped on days already practised, with different copy when a lesson is ready, and a 21:30 streak "last call" for adults only (never for under-18s). Decisions needed:
1. A desktop app is usually not running in the evening. `tauri-plugin-notification` can only notify while Ella is open (or minimised to the tray, which Ella does not do today). Is in-app-only acceptable, or should Ella register an OS-level scheduled task or login item? That is a larger, platform-specific change.
2. If built, the under-18 rule (no streak-loss messages) must carry over. The learner's age is already stored.
3. Where does the setting live? Profile → Settings is the natural place.

### P3: technical note on the island map

**8. Island map** (Mobile `supabase/functions/ella/islands.ts`, `schema/0022_islands.sql`, `app/lib/screens/island_home.dart`, `widgets/island/*`). Not implemented. Desktop would need:
1. **Its dependencies first.** Islands' goals count words made theirs (now available: `vocab::word_states`), Word of the day talks (available), **News talks** and the **Guess it** game. Desktop has neither of the last two. Umananda's last goal is Guess it, and Loktak's is News.
2. **Stops.** Each stop is two counted talks plus a lesson. Desktop already has the cadence (`lesson::LessonCadence`, `sessions.counted`, `lesson_ready`), so a stop count can be read as "lessons taken since the island was entered".
3. **A small table.** Mobile keeps one row per island stepped onto (`entered_seq`, `entered_words`, `crossed_at`, `stats`). Desktop would need the same (one learner, so no `learner_id`), because entering and crossing are events, not derivable state.
4. **Crossing at talk end**, inside the same transaction that commits the assessment, with the crossing returned on the summary for a recap card.
5. **UI.** The island Home, the map, the goals sheet, the islands list and the crossing recap card are all large, art-heavy pieces with no Desktop equivalent.
6. **Numbers.** Mobile's `ISLANDS` table is marked "first guess" and the feature is still behind a flag there. Porting it now risks chasing changes.

### Follow-ups worth doing
- Decide whether Desktop's level map should show levels under an unplaced learner's start as "not started" (Mobile's `open`) to match the badges.
- Show `SessionSummary.encouragement` / `headline`, or remove them. They are still unused.
- Consider a `messages.flagged` column so flagged answers earn no XP and never count as words (rustrict already keeps swear words out of My Words).
- Several Rust tests that depend on thread timing occasionally fail under a parallel `cargo test` run and pass alone, e.g. `a_judge_that_is_never_sure_lets_the_chat_run_to_twelve_answers`, `a_streamed_turn_says_its_talk_its_pieces_and_what_the_engine_did` and `model_queue::…an_assessment_held_between_its_judges_lets_nothing_less_urgent_in`. They are unrelated to this branch, but worth stabilising.
