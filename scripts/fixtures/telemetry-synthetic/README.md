# SYNTHETIC TELEMETRY: TEST FIXTURES ONLY

**Nothing here came from a laptop or a learner. Never copy these files into
`ella-telemetry`, its `inbox/`, `data/`, `ledger/` or `reports/`, or a relay.**

Made up by `scripts/telemetry-synthetic.py` to test the weekly report
(`telemetry:weekly`, report_version 4) end to end, through the same ingest
and report tools that `telemetry-repo.py` runs on real data.

How to tell it from real telemetry, at every level:

- every file, and every made-up laptop, is named `synthetic-…`;
- every event and every app batch runs app version `9.9.0-synthetic` or
  `9.9.1-synthetic`, which no build of Ella has, and the report shows it in
  "Turns by build" and the version breakdowns;
- every event is from 2 to 10 January 2026, before Ella sent any telemetry;
- the report names its tools `synthetic-fixture`, never an ella_tauri commit;
- the laptops' hardware is written `… (synthetic)` (only its class reaches
  the report).

## What is here

- `inbox/ingest-1/` to `inbox/ingest-3/`: the batches three ingests take in,
  received on 4, 7 and 11 January 2026. Six laptops: three Macs, two Windows
  laptops, and one Linux laptop running from source whose log was exported by
  hand (a manual batch).
- `expected/2026-W02.md` and `.json`: the report of 2026-W02 (5 to 11 January)
  against 2026-W01, made at 2026-01-12 06:00 UTC, as the tests expect it.

What the two weeks hold, so every section of the report has something to
show:

| Section | What happens |
|---|---|
| Usage | 6 installs, 1 new; talks of every type: free, lesson, placement, Word of the day |
| Talk lifecycle | talks completed; one left (sent noted `talk_left`); one left and then resumed; one unresolved (the manual export's); a talk of the week before whose recap comes on the Monday |
| Latency | the laptop slower than the cloud, and the week slower than the one before, so the triage flags; warm-ups, recaps, cold starts, with Ella's own model server and one it found running |
| Reliability | a "didn't hear you", a system error at `llm:start`, a recap that failed, the cloud down three times, an update check that failed, and a recap whose talk's turns never arrived |
| Collection | a seq that never arrives (win-01's 2), a batch sent at the third try, 3 events dropped with the queue full, the same batch twice (a duplicate), a manual batch, and a batch changed after it was made (refused) |
| Platform | 3 Macs shown by themselves; Windows and Linux folded into `small groups`; RAM and install place folded whole, as one group alone would be told from the rest |

## Using them

```bash
npm run telemetry:synthetic                 # prints the 2026-W02 report
npm run telemetry:synthetic -- --out DIR    # writes it as .md and .json
npm run telemetry:synthetic -- --write      # remakes this folder
npm run test:telemetry                      # scripts/test_telemetry_synthetic.py among them
```

The tests make the batches again and expect these bytes, take them in and
report, and expect `expected/`. A change meant to change the report fails
them until `--write` remakes this folder: then read the diff, as a reviewer
would, before committing it.

`.gitattributes` keeps every file here byte for byte: a batch's id is the
hash of its bytes.
