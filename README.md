# Ella desktop

A usable Ella vertical slice built with Tauri 2, React/TypeScript, Rust,
SQLite, and local AI engines. In local mode, the primary speech recognizer is
Canary-180M-Flash Q8_0 running in-process through `transcribe.cpp`; Whisper small
remains an HTTP fallback.

Canary is batch speech recognition, not true streaming. Ella runs a lightweight
stop-time VAD and optimizes the interval from microphone stop to final transcript.

## Prerequisites

- Node.js/npm, stable Rust, CMake, and the normal Tauri 2 platform prerequisites.
- Local llama.cpp and Piper assets under an engine root.
- The Canary and Whisper model files installed by the checked-in manifest.
- On the Windows x86-64 build host, Visual Studio C++ Build Tools and a Vulkan
  SDK providing `glslc`; on target machines, a current GPU driver for optional
  Vulkan acceleration. The packaged native runtime tries the Vulkan module
  automatically and retains the CPU module when Vulkan is unavailable.

The default development engine root is `engines/` in this repository — a symlink
or a staged copy of a built engine tree (`bin/`, `models/`, `piper-venv/`). Set
`ELLA_ENGINE_ROOT` to use another location. If you already have the tree that
`ella_flutter` builds, point at it once:

```bash
ln -s /path/to/ella_flutter/ella_app/build/engines engines
```

## Install and validate the STT models

From the repository root:

```bash
npm install
npm run models:install
npm run models:validate
```

`models:install` downloads the pinned Canary Q8_0 GGUF plus Whisper small from
`tooling/models.json`; `models:validate` checks the files' sizes and Canary SHA-256.
Native startup additionally checks the GGUF header, architecture, English/16 kHz
capabilities, and session creation. Failures name the bad file and print a repair
command. The model can also be checked without starting Ella:

```bash
cargo run --release --manifest-path src-tauri/Cargo.toml --bin stt-benchmark -- \
  --audio bench/fixtures/jfk.wav --duration-ms 4214 --iterations 1 --warmup 0
```

## User interface

The UI implements the **Ella Desktop** design (Claude Design project
`d4c02bb0-7d53-42ff-8c5b-ea4750e899ee`, file `Ella Desktop.dc.html`). The
design's window chrome — title bar and traffic lights — is left to the OS.

- Tokens, type and geometry live in [`src/styles.css`](src/styles.css).
  Fonts are bundled under `public/assets/fonts` because the Tauri CSP is
  `font-src 'self'` — nothing is fetched from Google Fonts at runtime.
- Ella herself is drawn in CSS, not illustrated: see
  [`src/components/EllaMascot.tsx`](src/components/EllaMascot.tsx). The design
  draws her afresh for each placement, so each placement is a variant whose face
  is a block of custom properties on `.ella--{variant}`. She holds still, and
  so do the talk partners and every screen: each pose is a class the
  stylesheet draws, changed only when what she is doing changes. On the talk
  stage her ears lift while she listens, her eyes close into lines while she
  thinks, and her mouth opens into the design's "o" while she talks; a
  partner glances up to think and opens their mouth to talk. Nothing runs on
  a clock: see [Latency on laptops](#latency-on-laptops).
- Onboarding is the five-step flow — welcome, name, age, mic check, placement
  chat — in
  [`src/components/OnboardingFlow.tsx`](src/components/OnboardingFlow.tsx). The
  mic check and the placement chat open the real microphone. The profile can run
  the mic check again. The placement chat finds the learner's level: see
  [Levels](#levels-the-placement-chat-and-moving-on).
- Then the sidebar screens: **Home**, **Talk partners** and the **Profile**
  (from the name at the foot of the sidebar, which stays lit on the profile and
  the level map behind it), plus the conversation, its recap, and **Levels and
  badges**. A conversation and its recap hide the sidebar and fill the window;
  Space works the microphone.
- Talk partners are real where the backend is: Bippo's and Grumble's goals start
  the chores in `chores()` through `start_chore`, which plays the character and
  keeps the score. Dr Wobble's goal opens the doctor topic, and Zig's debate has
  no chore yet, so it shows as coming soon. Goals a learner is too young for are
  left out, mirroring `catalog_for`. The four partners are drawn in CSS, as the
  design draws them, and the recap and a badge's sheet reuse the same drawings.
- The **profile** is the design's: the learner in the colour they gave their
  Ella, with her rising out of the card's corner, their age and the day they
  joined; the day streak, talks done and time spoken; the settings; MY LEVEL
  with a track of all six levels; and the badges, those earned latest first and
  three to earn next. "Where to earn more" and MY LEVEL open **Levels and
  badges**: the path of levels with the anytime badges beneath it, and the
  picked one's page — its goal, its five steps as bars with a segment per
  skill, and the badges filed under it. Where the design writes a CEFR code
  ("A2", "to B1"), the window writes the level's number, as Ella Mobile does.
- **Badges** are read off the learner's history, in
  [`src/lib/presentation.ts`](src/lib/presentation.ts) (`learnerBadges`): Hello,
  Ella (a placement chat finished), First talk, Bargainer (the stall price talked
  down, or the free bargaining talk finished), Deposit back (Grumble's refund
  goal met), the 3-, 7- and 30-day streaks, and 50 talks. Each is dated by the
  talk that first earned it, and a scene's badge counts the goes at it. A badge
  opens a sheet saying how it is going, what earns it and where; its mic opens
  that partner's scenes on Talk partners, or starts the talk that earns it.
  Only what can be counted is shown, as on Ella Mobile. The design's "Sold!",
  "Clear patient" and "Take a stand" are left out until something judges the
  pen's rubric, Dr Wobble has a scene with a goal, and Zig has a scene at all;
  its level-4 and mystery badges stand for scenes nobody has made yet. A scene
  can be played at any level, so nothing is locked: a badge sits at the level
  the design files it under, and a scene the learner is too young for leaves its
  badge out.
- A laptop keeps one learner, with their talks, progress and avatar colour, all
  in the local SQLite database. The profile edits them through `save_learner`
  and saves the avatar colour through `save_avatar_color`. "Log out"
  (`log_out`) only signs out and deletes nothing. "Log in" (`log_in`) then
  greets the saved learner by name, in their own colour, and "Take me in"
  brings back every talk, the streak and the badges; on a laptop nobody has used
  yet it asks for a name and goes straight in. "Let's start" after a log out
  is the same learner onboarding again: `save_learner` signs them in and their
  history stays theirs.
- The design shows framing the Rust backend does not model yet — per-topic
  category and duration, a mentor lesson, the badges' names and colours, the
  cast's names. All of it is resolved in
  [`src/lib/presentation.ts`](src/lib/presentation.ts), which derives what it can
  from `AppSnapshot` and marks the rest `PLACEHOLDER`. The streak, the week
  strip, talks, answers and time spoken, and every badge all come from
  `AppSnapshot.progress`, which the backend sums over the learner's whole
  history: its days, and every finished talk in the order they ended with the
  day each ended and whether it met a ledger goal. A day counts once the
  learner has answered on it. The home screen's weekly talk count includes
  every talk with an answer, on the day of its first answer; the profile's
  talks done and the badges count only talks finished with at least one
  answer. Time spoken is the length of each spoken answer, kept with it
  (`messages.spoken_ms`): the speech the VAD kept, or the whole recording of a
  streamed turn. Typed answers, and every answer from before Ella kept the
  length, have none, so until one is measured the profile shows "—" and Home
  shows answers spoken rather than a time nobody measured. When the backend
  grows a field, delete the constant and read the snapshot instead.

The intended window is 1440x900; the layout holds down to the 1240x740 minimum,
with the tallest mascots scaling down on short windows.

## Levels, the placement chat and moving on

The same curriculum and rules as Ella Mobile, run on the laptop. Learners never
see a CEFR code: the window shows a level's name ("Finding My Voice") and its
number on the ladder, 1 to 6.

- **The curriculum** is Ella Docs' six levels, A0 to C1, of five steps each —
  117 skills, word for word, with Ella Mobile's short labels. It lives in
  [`shared/curriculum.json`](shared/curriculum.json), which the backend
  ([`curriculum.rs`](src-tauri/src/curriculum.rs)) and the browser preview
  ([`src/lib/curriculum.ts`](src/lib/curriculum.ts)) both read. Everyone
  starts at Step 1 of A2 until a placement says otherwise.
- **The placement chat** replaces the one-question first talk. Ella climbs from
  easy questions (themselves) to harder ones (the past, an opinion and why,
  something imagined), and the chat ends by `progress::placement_ends`, as on
  the phone: never before the fifth answer, from the fifth when the model is
  ready and highly confident, from the ninth when it is fairly confident, and
  at the twelfth regardless. Its last turn is a goodbye and closes the session;
  the level is read while Ella says it, and the learner starts at Step 1 of it.
  A chat ended before it heard five answers — Skip, or End talk on one left
  open and picked up again from Home — places nobody. A learner who never had
  one — everybody upgrading from 0.1.7 — is offered it on the level map, even
  after moving on through talks. Once a learner stands anywhere above the
  start, a placement only ever moves them up.
- **Every talk quietly aims at one skill** of the learner's step, picked when it
  starts by the phone's priority (need, uncertainty, review due, curriculum
  order, variety, and a cooldown for the latest aims) and kept on the session.
  Unlike on the phone, the aim is not named in Ella's instructions. Told it,
  Ella's 3B model asked about the skill instead of the topic: aimed at
  collocations ("make a decision, pay attention, take a risk"), the job
  interviewer brought those words into 14 of 15 questions, and aimed at the
  present perfect it said the skill's example, "I've lived here for 3 years",
  as its own. Without the aim, none of the same 30 replies did (see
  `ella_system_prompt`). The prompts pitch Ella's words at the learner's level
  instead of the old fixed A1, and a talk keeps the level it began at, so its
  instructions stay in llama.cpp's cached prefix even when another talk's
  assessment moves the learner on meanwhile. A free talk's instructions are
  the same for every talk on the topic at that level and can be kept on disk
  between talks: see [Latency on laptops](#latency-on-laptops).
- **Ending a talk scores it** (`assess_session`) on the skills of the step and
  the next. A demonstration the judge is at least 0.70 sure of raises that
  skill's mastery; a skill is owned at 0.75, shown in two talks on two topics.
  A step is done when its vocabulary and fluency skills are owned and its
  grammar skills have started to show; then the learner moves to the next step,
  or after Step 5 to Step 1 of the next level. If every skill left has missed
  twice running, the next step's skills join in. See
  [`progress.rs`](src-tauri/src/progress.rs).
- **The recap** (the Ella Desktop Recap design) follows every talk across the
  whole window: Ella cheering, or for a chore the talk partner with whether the
  goal was met, its badge and their last line; then Ella's notes — what went
  well, one fix the learner can hear said right, and the skills that grew — the
  streak with today counted, and tomorrow's topic. "Step complete!" or "Level
  up!" shows under the headline when a talk moved them on. It opens at once and
  the notes fill in when the model has read the talk. The app owns that
  question, so leaving early loses nothing: Home catches up, and a step or
  level finished meanwhile is celebrated in a toast. Home and the profile carry
  a MY LEVEL card, and both open the level map, as the recap's footer does.
  Meeting a ledger chore's goal earns its badge: Bargainer for the stall price,
  Deposit back for the refund.
- **Ella's notes** ([`notes.rs`](src-tauri/src/notes.rs)) need three answers
  and twelve words; a shorter talk says so at once. What went well is read off
  the learner's own words by rules (reasons given, the past told, questions
  asked, politeness, full sentences), so it holds with or without a model. The
  fix is the model's correction of the answers, compared word by word with what
  was said, so the phrase quoted is always the learner's own. A change that
  only writes the same words another way is not a fix: "every day" for
  "everyday", "favorite" for "favourite", "there" for "their", "two" for "to"
  or "25", "I am" for "I'm". The answers were heard, not written, so their
  spelling is the speech recognizer's, and such a fix showed the learner their
  own words twice. It is kept with
  the assessment; a correction that cannot be read leaves the notes without a
  fix rather than failing the talk, and "Nothing to fix" is only said when a
  model looked. A ledger chore's recap is read off its ledger (`ChoreRecap`),
  which is also what earns the Bargainer badge now: the stall price talked
  down, or the free bargaining talk finished. Deposit back is earned the same
  way, by Grumble's refund goal met.

Assessments are worked out once and kept on the talk (`sessions.assessment`),
in one transaction with the skills and the level they change, so asking again
is instant and counts nothing twice. A model whose answer cannot be read is an
error the summary offers to retry, never a talk that silently counts for
nothing. Without a model (demo mode, the browser preview) the placement runs to
five answers and places at A2, and talks are kept unscored.

**Asking a 3B model to judge.** The phone asks a much larger model, and three
of its prompts had to change to work with Qwen2.5-3B. Each was measured against
the model on sample transcripts, and the prompts' doc comments keep the numbers:

- *Has the placement heard enough?* This is asked after each exchange from the
  fourth, on its own thread while the reply is played. The next answer reads
  the verdict, so it judges the answers before that one. The phone's model
  judges the latest answer too, but only because it writes the verdict in the
  same reply; here that would put a model call in front of every turn. The chat
  still ends between the fifth answer and the twelfth, and the level is read
  off all of them. It is sent as an aside
  after the chat under the chat's own system prompt, so llama.cpp answers it
  from the slot's cached prefix: about 100 prompt tokens and half a second on
  an M-series Mac, where a prompt of its own would evaluate the whole chat
  again, and the next turn's cache would go with it.
- *Which level?* The phone's reply template shows `"level":"A2"`, and the 3B
  model copied it: every sample came back A2, from single words to fluent. With
  a placeholder in the template, and only the learner's answers to read, the
  samples came back A1, A2, B2, B2 — in order, and at most one level out. The
  phone's "when the sample is very short, do not guess high" is enforced by
  counting instead (`progress::level_ceiling`). Answers averaging under three
  words stay at A0, under five at A1, and under seven at A2.
- *Which skills did the talk show?* Asked for scores alone, the model marked
  six skills of eight at 0.9 for a talk of "yes", "samosa" and "good. I like".
  So it now has to quote the learner's own words for every skill it claims, and
  a claim counts only if they really said it, with one skill per quote. For
  that talk it now claims nothing.
- *What is the one fix?* Asked for the one mistake and its correction, the model
  "fixed" "600 is too much" into "600 is too high", and "yes" into "Did you eat
  anything today?". Asked to rewrite every answer changing as little as it can,
  it left fluent, casual and bargaining talks word for word, and found "it
  have", "we plays", "is best player" and "I go to market". It still swaps a
  right word for another now and then: "give it for 350" came back as "give it
  to 350", "their house" as "the house", "one small stall" as "a small stall".
  So a change is only shown when a grammar fix makes it: a word put in its
  right form ("go" to "went", "me" to "I", "buyed" to "bought"), or small
  words only added or only dropped (`notes::fixes_grammar`). A JSON schema pins
  the answer to one line per answer; without it the model sometimes wrote each
  answer twice, as given and corrected. Asked for praise, it described the
  topic instead ("went to market"), and picking from a numbered list it chose
  the same two for every talk — which is why what went well is rules, not the
  model.

To run the whole flow against a real model, start llama-server with Ella's
model and run the ignored end-to-end test:

```bash
ELLA_LLM_BASE_URL=http://127.0.0.1:39091/v1 \
cargo test --manifest-path src-tauri/Cargo.toml --lib live_model -- --ignored --nocapture
```

## Run the complete local voice POC

macOS development host:

```bash
cd /path/to/ella_tauri
npm run local
```

Or keep sidecar logs in a separate terminal:

```bash
# Terminal 1
npm run engines:local

# Terminal 2
npm run desktop:dev:local
```

`desktop:dev` is the UI-only launch: it leaves `ELLA_ENGINE_MODE` unset, so the
app runs the demo engine and **has no speech recognition** — the desktop webview
does not provide the Web Speech API, so voice turns can only fail with
"native speech recognition is not enabled in demo mode". Use
`desktop:dev:local` (or `npm run local`, which starts the engines too) whenever
you need to speak to Ella. The mode is no longer surfaced in the UI — read it
back from `ELLA_ENGINE_MODE` or the startup log.

Windows x86-64 development host (PowerShell):

```powershell
cd C:\path\to\ella_tauri
npm install
npm run models:install
npm run models:validate
.\scripts\run-local-poc.ps1 -EngineRoot C:\path\to\ella_tauri\engines
```

The native voice route is:

```text
WebAudio PCM -> stop-time energy VAD -> SpeechToTextEngine
                                      -> Canary/transcribe.cpp (primary)
                                      -> Whisper HTTP (fallback)
             -> streamed llama.cpp -> Piper (per sentence) -> WebView playback
```

`SpeechToTextEngine` and `SttRouter` keep the STT boundary independent of the
Canary adapter, so a future Parakeet streaming adapter does not change the
application or UI contracts.

Piper runs a sentence behind the language model rather than after it. Whole
sentences leave the token stream as they complete and are synthesized on a
worker thread, so Piper's time overlaps the model's instead of following it —
about 300 ms off a turn. None of it is heard until the whole reply is written
and the turn saved: released sentence by sentence as the model wrote them, Ella
started talking sooner still, but the text then arrived in pieces, and a reply
that gains words cannot be centred without the words being read jumping as
each piece lands. Once the turn is saved, though, the sentences already
synthesized go to the window at once and the rest follow as Piper finishes
them, rather than all of them waiting for the last one. The first carries the
whole reply's text (`SpeechStreamEvent.reply`), so the window shows all of it,
centred, the moment she starts, and the turn that arrives a moment later puts
the same words in the conversation (`TurnResult.streamed_segments` says it is
already playing). Ella's opening streams from the start, because its text is on
screen from the moment the conversation opens, so there is nothing to stage —
`speak_opening` is a separate command from `start_session` so the screen
appears before Piper is asked for anything.

A turn that could be thrown away is safe either way: a ledger chore may
regenerate its reply when the character breaks its own limit, and the audio is
only reused when it says exactly what the reply says. A reply said differently
from what Piper read is said afresh, sentence by sentence in the same way, and
Piper stops reading the one that was dropped. Nothing the learner hears is ever
retracted.

The reply carries word timings, so the word being spoken is highlighted and the
words ahead of it are dimmed. `infrastructure/speech_timing.rs` estimates them
from syllables, characters and punctuation, anchored to the sentence's exact
duration and to the leading and trailing silence measured off the PCM. Fitted
and measured against real Piper phoneme alignments: onset error mean ~70 ms, p90
~160 ms, roughly three quarters of words inside 100 ms. The words stay an
estimate even though Piper's own sound timings reach the window (below): the
standalone binary reports none, and espeak merges words ("in the" becomes one
phoneme group) in about 8% of sentences, which leaves no safe positional mapping
back to the text. The highlight reads the audio clock at each word's start and
end, a few times a second, rather than on every frame.

### Piper's own timings

The resident daemon
([`piper_daemon.py`](src-tauri/src/infrastructure/piper_daemon.py)) loads the
voice with its sampled token durations marked as a second graph output — the
one change `piper.patch_voice_with_alignment` makes, done to the model's bytes
in memory, so it needs neither the 68 MB `onnx` package nor a patched copy of
the voice, and works for any Piper voice. The NavGurukul voice's session loads
in the same ~0.7 s as before, plus ~35 ms to read and patch the model; a voice
the patch cannot read loads as it always did, without timings. Every response
then carries each token's `[symbol, samples]` — sounds, stress marks, word
spaces, Piper's blanks and the sentence's `^`/`$` — and only if they add up to
every sample of the audio. `phoneme_spans` in `speech_timing.rs` turns them into
`PhonemeSpan`s, on every streamed sentence (`phonemes`) and on the whole reply
for replay (`speech_phonemes`).

The window used to move Ella's mouth with them, sound by sound, as Ella Mobile
does. It no longer does: a mouth redrawn every frame while she talked ran on
the same CPU as Piper reading the rest of the reply, and on a talk's first line
as the model evaluating its instructions. Her mouth opens into the design's "o"
while she talks, as it always did for a voice with no timings.

Running a live check needs piper-tts and a voice:

```bash
ELLA_PIPER_PYTHON=<python with piper-tts 1.8> ELLA_PIPER_VOICE=<voice.onnx> \
cargo test --manifest-path src-tauri/Cargo.toml --lib live_piper -- --ignored --nocapture
```

Environment overrides:

- `ELLA_ENGINE_ROOT`
- `ELLA_LLM_BASE_URL`
- `ELLA_STT_ENGINE` (`canary` or `windows`; defaults to `canary` everywhere, with Windows Speech Recognition as the fallback on Windows; `windows` makes it the primary with Canary as the fallback)
- `ELLA_PIPER_BINARY` and `ELLA_PIPER_VOICE`
- `ELLA_PIPER_DAEMON=0`, which starts Piper afresh for every line, as Windows
  did before it stayed resident
- `ELLA_LLAMA_THREADS`, llama.cpp's threads when Ella starts the server; one
  per physical performance core by default
- `ELLA_LLM_SLOT_DIR`, where a llama-server someone else started with
  `--slot-save-path` keeps its saved slots, so Ella can use them too; the
  development scripts start theirs with `engines/models/llm-slots` and set it
- `ELLA_CANARY_MODEL`, `ELLA_STT_THREADS`, `ELLA_CANARY_VERIFY_SHA256`

## Timing telemetry

Every native turn writes one JSON line with `event: "ella_turn_latency"` and a
correlation ID. Voice lines contain audio input/after-VAD duration, VAD, STT,
Canary mel/encode/decode, STT engine/backend/fallback, LLM TTFT/completion, Piper
first-audio/completion, when Ella started speaking (`speech_ms`: her first
sentence handed to the window, or the total for a reply the window plays whole),
total latency, and success/error status. The WebView also logs
`ella_voice_playback_ready` after audio reaches the browser playback path.

The lines are kept in `telemetry/latency.jsonl` in the app's data folder, on
every laptop. `npm run telemetry:report -- <path>` prints each day's medians and
95th percentiles, including when Ella started speaking ("speaks"); a log from
before `speech_ms` counts its total there. A talk's first turn is the one that
waits for the talk's instructions to be evaluated, and shows it in its
`llm_ttft_ms`.

## Latency on laptops

Most of the laptops Ella runs on have no GPU llama.cpp can use, so the model
runs on the CPU and a turn is mostly the model. Measured on an M1 Pro held to
four CPU threads — a classroom laptop is slower — a free talk's turn evaluates
20-95 new prompt tokens and writes 12-26, and a chore's evaluates 62-152. The
talk's instructions, which every turn of it shares, are 1,068-1,196 tokens,
evaluated once when the talk opens, and its first reply waits for them.

- **Piper stays running on every platform.** The macOS bundle's Python runs
  `piper_daemon.py`. The Windows bundle's standalone `piper.exe` is started once
  with `--json-input`, keeps the voice loaded, and answers each JSON line by
  writing that line's WAV; the WAV is read once the file is as long as its own
  header says, because `piper.exe` names it before it closes it. Before, Windows
  started `piper.exe` and loaded the voice for every line Ella said, after the
  model had finished writing, and a reply got no overlap with the model: on the
  M1 Pro a one-shot Piper took 0.9-1.2 s for a reply a resident one says in
  0.12 s. A Piper that cannot run resident, or leaves a line unanswered for
  20 s, is set aside for the session, and every line starts Piper afresh, as
  before.
- **Nothing on screen moves.** Ella, the talk partners and every screen hold
  still: no keyframes or transitions in the stylesheet, no Web Animations, no
  animation frames. Before, the window ran Ella's body loops, her blinking,
  grinning and eyes that followed the pointer, a lip-synced mouth redrawn every
  frame while she talked, a partner's face redrawn every frame for as long as
  their talk was open, and confetti on the recap, all on the CPU the model,
  Canary and Piper were using. The talk screen also re-rendered for every block
  of microphone audio, about 23 times a second, to keep a level nothing on
  screen showed; it now keeps the level only for the "speak up" nudge, without
  a render. The word highlight wakes at word boundaries instead of every frame.
- **The llama.cpp build is pinned** (`ELLA_LLAMA_TAG` in
  [`release.yml`](.github/workflows/release.yml)). Every release used to ship
  whatever llama.cpp was newest that day: b11317 in 0.1.9, b11321 in 0.1.11,
  b11403 in 0.1.12, b11405 in 0.1.13. A kept talk's file name carries the
  server's build (below), so every update threw away every kept talk, and the
  first reply of each topic waited for its whole instructions again. On the
  Windows test laptop the first reply took 0.9 s on 0.1.11 and 21-26 s on
  0.1.12. The three builds ran the same on the CPU, with Ella's flags and four
  threads on an M4: 125-131 prompt tokens a second over a talk's instructions,
  134-137 on `llama-bench`'s pp256 and 35 written. Bump the pin on purpose, and
  expect one slow first reply per topic after that update.
- **Ella starts talking once the reply is written and saved**, not once its
  last sentence is synthesized (see above). On the M1 Pro that is 5 ms after
  the model's last token instead of 95-130 ms; on a laptop, with a slower Piper,
  it is more.
- **Threads.** llama.cpp gets one thread per physical performance core. It had
  every logical core but two, which is six threads on a four-core laptop with
  hyper-threading; on the M1 Pro's CPU, spreading from its six performance cores
  onto the two efficiency cores cut decode from 47 to 34 tokens a second. The
  daemon's Piper uses two threads that sleep between sentences: onnxruntime's
  default, a spinning thread per core, cost the model 12% of its decode speed
  while Piper worked beside it, for 30 ms less per sentence.
- **A talk's instructions are kept on disk.** A free talk's are the same for
  every talk on the topic at the learner's level. A chore's are the
  same for every talk of it at that level, and the placement chat's for the
  learner, because everything that moves between turns is in the turn's note.
  The first talk of each evaluates them and has llama-server save the slot
  (`--slot-save-path`, in `models/llm-slots`, the six most recent at 23-45 MB
  each). Every later one, after a restart too, restores them in milliseconds and
  evaluates only its opening. (The timings below were measured when a free
  talk's aim still came after its topic's words and was evaluated with the
  opening.) On the M1 Pro's CPU a talk on a topic
  done before warmed up in 1.7 s instead of 10 s, and its first reply, answered
  at once, came in 1.3 s instead of 10.2 s. A talk's replies wait for its
  warm-up rather than slip in between its requests, so a slot is never saved with
  a conversation in it, and a slot's file name carries the model file, its size
  and date, and the server's build, so it is only ever restored into the model
  and server that made it.
- **A talk goes first** (`ModelQueue`). llama-server answers one request at a
  time, and the recap's scoring and fix (or a placement's level) used to go to
  it the moment a talk ended. A learner who went straight on to another talk had
  its warm-up queued behind them, and a judge could land between the warm-up and
  the first reply and take the talk's instructions out of the slot, so the
  first reply evaluated them again. Now everything a learner is not waiting on
  is an errand: it runs only while no talk is open (a talk is open until it is
  finished, or 3 minutes after its last request), reads its prompt in pieces of
  about 400 characters, streams its answer, and gives way at the next piece
  when a talk wants the model, carrying on once the talk is over. Pieces,
  because llama-server reads a request's whole prompt before it looks at
  anything else: on the M1's CPU a request cancelled 2 s into a 1,400-token
  prompt held the next one for 10.6 s, and the same prompt read in pieces held
  it for 0.4 s; a dropped stream frees it in 0.4 s too. A smaller server batch
  would have done the same for every request at a cost of 5-10% of prompt speed.
  On a llama-server held to one CPU thread, about a classroom laptop
  (31 prompt tokens a second, 16 written), a chore opened 3 s into the last
  talk's scoring and answered 8 s later had its first token after 38 s instead
  of 67 s the first time, which is its own 1,139 tokens of instructions, and
  after 2.9 s instead of 90 s the second, when it restored them: in the 90 s,
  the fix had run between the chore's warm-up and its first answer, and the
  answer evaluated 1,226 of its 1,230 prompt tokens.
- **Home's talk is got ready ahead.** Whenever the window reads Home (after
  onboarding saves the learner, a log in, or a talk), the engine is asked to get
  the talk it offers first ready: the placement chat for a learner who has not
  talked yet, else "Today's talk", the first topic, at their level. It is an
  errand like the scoring, after it in line, so it runs while the learner looks
  at Home or the recap, is dropped the moment a talk opens, and costs a look at
  a file once that topic's instructions are kept. On the one-thread server it
  was ready 41 s after it was asked for, and the talk then restored 1,092
  tokens in 15 ms and had its first answer's first token after 1.2 s, against
  a 39 s wait for the warm-up of a first talk on a topic.

Tried and not kept:

- Putting the rules and guardrails every talk shares ahead of the scene, so a
  new topic would restore them too and evaluate about half its instructions.
  In the bench's safety script Ella then accepted a hug in four of twelve
  replies where the prompt as it is had accepted none. Moving only the aim to
  the end made no difference: from the same conversation, sampled 100 times at
  each hug, the prompt as it was and the one with its aim last accepted 1 and 0
  times, and from a conversation that had drifted, 4 and 3.
- llama-server's `--cache-reuse`, which lets a chore keep the previous exchange
  instead of evaluating it again after the turn's note. It saved 20-30 tokens a
  turn, and eight bench runs of the two ledger chores broke the ledger 5 times
  with it against 4 without: too few runs to tell, and too little saved to try
  it on learners.

## Repeatable Canary/Whisper benchmark

Start the Whisper fallback, then run the fixed 4.214-second same-input test:

```bash
npm run engines:local
```

```bash
ELLA_ENGINE_ROOT="$PWD/engines" \
npm run benchmark:stt -- \
  --audio bench/fixtures/jfk.wav \
  --duration-ms 4214 \
  --whisper-url http://127.0.0.1:39092 \
  --warmup 1 \
  --iterations 5 \
  --output bench/results/local.json
```

The tool reports sorted samples, medians, transcripts, the live same-input
comparison, and the supplied 3387 ms Whisper baseline comparison. Details and
the development-host result are in [`bench/README.md`](bench/README.md).

## Test and build

```bash
npm run check

# Full local voice path: VAD -> native Canary -> llama.cpp -> SQLite -> Piper
ELLA_ENGINE_MODE=local \
ELLA_ENGINE_ROOT="$PWD/engines" \
cargo test --release --manifest-path src-tauri/Cargo.toml \
  application::tests::complete_local_voice_turn_uses_canary_and_returns_playable_audio \
  -- --ignored --nocapture

# Native application build without an installer
npm run desktop:build
```

The ignored end-to-end test requires healthy llama.cpp and Whisper sidecars;
Whisper is present to verify the fallback is ready, while the assertion confirms
the turn actually used Canary and returned playable Piper audio.

## Windows packaging

Collect Windows x86-64 llama.cpp, whisper.cpp, and Piper executables plus models
into an engine root, then stage and build on Windows:

```powershell
.\scripts\stage-windows-engines.ps1 -EngineRoot C:\ella\engines
rustup target add x86_64-pc-windows-msvc
npm run tauri -- build --target x86_64-pc-windows-msvc
```

The Windows Cargo target enables `transcribe-cpp`'s `vulkan` and
`dynamic-backends` features. `build.rs` stages the produced runtime, CPU, and
Vulkan DLLs next to the packaged executable; Tauri also bundles the staged
engine tree. Run the resulting build on both a Vulkan-capable and CPU-only
Windows 11 x86-64 machine before distribution.

## Releases and automatic updates

Windows installers are built by [`.github/workflows/release.yml`](.github/workflows/release.yml)
on a `windows-latest` runner, because the MSVC target cannot be
cross-compiled from the development Mac. Pushing a `v*` tag builds the
installer, signs the update bundle, and attaches both — plus `latest.json` —
to a **draft** GitHub release.

```bash
# keep the three version numbers in step: package.json, src-tauri/Cargo.toml,
# src-tauri/tauri.conf.json
git tag v0.1.1 && git push origin main --tags
```

Publishing that draft is what ships the update: installed copies poll
`releases/latest/download/latest.json`, and a draft release is not "latest".
That is the release switch — build first, decide later.

Every installed copy checks once at launch, before there is a conversation to
interrupt, and applies what it finds
([`src/lib/updates.ts`](src/lib/updates.ts)). An update that fails to verify
against the public key in `tauri.conf.json` is discarded and the installed
version keeps running.

The private half of that key is **not** in the repository. It lives in the
`TAURI_SIGNING_PRIVATE_KEY` repository secret and in `~/.ella-updater/` on the
machine that generated it. Losing it means no existing install can ever be
updated again — they would each have to be reinstalled by hand — so it belongs
in a password manager, not only on one laptop.

Two things the pipeline deliberately does not solve:

- **Code signing.** Builds are unsigned, so SmartScreen warns on first run
  ("More info" then "Run anyway"). An Authenticode certificate is weeks of
  lead time and is the thing to start early; it is unrelated to the update
  signing key above.
- **The Indian voice.** `en_IN-navgurukul-medium` is not publicly
  downloadable, so CI stages it from the `ELLA_PIPER_VOICE_URL` secret. Without
  that secret the build still succeeds and ships the public `en_US` voice,
  which is the fallback Ella already has.

## What the installer carries, and what it fetches

The installer is ~200 MB: llama-server, Piper and the Piper voice, plus Ella
herself. The weights are not in it — ~2.3 GB against a 2 GB cap on a release
asset, and bundling them would make every one-line fix a multi-gigabyte
download.

Instead [`infrastructure/models.rs`](src-tauri/src/infrastructure/models.rs)
fetches them into app data on first launch, from the same
[`tooling/models.json`](tooling/models.json) the development tooling reads,
compiled into the binary. Downloads resume after an interruption, Canary is
checked against the SHA-256 the manifest pins, and a file is only given its
real name once it is complete — a half-downloaded model can never be mistaken
for a usable one.

This is also how a **model change ships**. Every downloaded file is recorded
beside the weights with the variant and URL it came from. Point the manifest at
a different GGUF, cut a release, and the next launch notices that what is on
disk is no longer what the manifest asks for and fetches the new one. No
separate mechanism, and no version number to bump by hand.

The window opens immediately while all of this happens behind it: the engine
arrives underneath a placeholder
([`infrastructure/engine_manager.rs`](src-tauri/src/infrastructure/engine_manager.rs)),
so a learner can enter their name and check their microphone during the
download, and only the talking itself waits.

`llama-server` is started and supervised by the app in an installed build —
free port, readiness poll, killed on exit. The development scripts are
unchanged: setting `ELLA_LLM_BASE_URL` means someone else owns the server, and
Ella will not start a second one.

## Where a learner's data is kept

The learner's profile, talks and answers are kept in one SQLite database,
`ella.sqlite3`, in the app's data folder
(`~/Library/Application Support/org.navgurukul.ella.desktop/` on macOS,
`%APPDATA%\org.navgurukul.ella.desktop\` on Windows). None of it leaves the
laptop, and nothing in the app deletes it: "Log out" only signs out. While Ella
runs, the newest writes sit in `ella.sqlite3-wal` beside it, and quitting folds
them into `ella.sqlite3`; to back up or move a tester's data, quit Ella and copy
every `ella.sqlite3*` file together. Every saved turn is synced to the drive
before Ella moves on (`synchronous = FULL`, plus `fullfsync` on macOS), so it
survives a crash, a force-quit or a battery that dies.

`models/llm-slots/` beside it holds llama.cpp's saved slots: the evaluated
instructions of the last six talks' topics, chores and placement chat, and of
the talk Home offers next (see [Latency on laptops](#latency-on-laptops)). They name the learner, since the
instructions do, but hold nothing they said. Deleting them loses nothing: the
next talk on a topic evaluates its instructions again and saves them afresh.

A development build has the same identifier, so it opens the same database and
sees the installed app's learner. Opening a v0.1.6 database only adds columns
and one table, so v0.1.6 can still open it afterwards. The learner gains the
avatar colour, whether they are signed out, their level and step, and whether a
placement has read a level for them. Each session gains whether it was the
placement chat, the skill it aimed at, the level it was pitched at and its kept
assessment. Each answer gains how long it lasted when it was spoken
(`spoken_ms`), empty for a typed one. The new table is `skill_mastery`, one row
per skill a talk has been scored on. It is a new name on purpose, because the first releases' garden
left a `skill_progress` table of its own on some laptops. `level_name` keeps
the learner's real level name, which is what v0.1.6 shows. A database from an unreleased development
build that briefly kept several learners on one laptop is converted back once:
it keeps the learner who was signed in, or else whoever was about most recently
(still signed out), and every talk. Before running a development build on a
tester's laptop, quit the installed app and back up every `ella.sqlite3*` file
all the same. The browser preview keeps its own, separate copy in the browser's
local storage, and reads what older previews left there the same way.

Once the models are downloaded on the first launch, Ella needs no internet: the
speech, language and voice engines all run on the laptop, and the update check
gives up quietly when offline. If the models cannot be refreshed — no internet
after an update that names a newer model, or a lost download record — Ella
loads the weights already on disk and tries again on the next launch.

## POC boundaries

The automated full-turn test feeds a real WAV fixture into the application
service, rather than operating physical microphone/speaker hardware. Windows
cross-compilation and GPU execution are not available on the current Intel Mac,
so Windows installer, driver fallback, microphone permission, and hardware
playback still require the two-machine checks above. Model update UI, signed
installers, native capture, true streaming STT, and Parakeet remain outside this
POC.
