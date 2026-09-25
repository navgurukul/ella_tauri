use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::{
    domain::{DayActivity, Learner, LearnerProgress, Message, Session, SessionListItem},
    error::{EllaError, EllaResult},
};

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> EllaResult<Self> {
        let connection = Connection::open(path)?;
        let database = Self {
            connection: Mutex::new(connection),
        };
        database.migrate()?;
        Ok(database)
    }

    #[cfg(test)]
    pub fn in_memory() -> EllaResult<Self> {
        let database = Self {
            connection: Mutex::new(Connection::open_in_memory()?),
        };
        database.migrate()?;
        Ok(database)
    }

    fn connection(&self) -> EllaResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| EllaError::Database(rusqlite::Error::InvalidQuery))
    }

    fn migrate(&self) -> EllaResult<()> {
        let connection = self.connection()?;
        // `synchronous = FULL` syncs the write-ahead log on every commit, so a
        // saved turn survives the app crashing or being killed straight after.
        // It is already SQLite's default; it is spelled out so nobody trades
        // it away for speed without seeing what it buys. On a Mac a plain sync
        // can still sit in the drive's own cache, so a battery that dies or a
        // held power button could take the last few turns with it; `fullfsync`
        // makes each commit, and each checkpoint, wait until the drive has
        // them. That is one short wait per saved turn, and Windows and Linux
        // ignore both pragmas.
        //
        // The tables are v0.1.6's, statement for statement, so somebody who
        // goes back to that release finds a database it can still read and
        // write. Anything added since is added the same way `age` was.
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             CREATE TABLE IF NOT EXISTS learner (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               name TEXT NOT NULL,
               age INTEGER,
               level_name TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               topic_id TEXT NOT NULL,
               topic_label TEXT NOT NULL,
               status TEXT NOT NULL CHECK (status IN ('active', 'complete')),
               started_at TEXT NOT NULL,
               completed_at TEXT
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               speaker TEXT NOT NULL CHECK (speaker IN ('learner', 'ella')),
               content TEXT NOT NULL,
               turn_number INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_session_turn
               ON messages(session_id, turn_number, created_at);
",
        )?;
        rename_legacy_speaker(&connection)?;
        add_learner_age(&connection)?;
        widen_message_speaker(&connection)?;
        add_chore_tables(&connection)?;
        // Before `add_learner_profile`: the learner table it rebuilds is made
        // with those columns already in it.
        keep_one_learner(&connection)?;
        add_learner_profile(&connection)?;
        Ok(())
    }

    /// The learner saved on this laptop, if anybody has told Ella their name,
    /// and whether they are signed in. A log out keeps them here, with
    /// everything they did, for the next "Log in".
    pub fn learner(&self) -> EllaResult<Option<(Learner, bool)>> {
        let connection = self.connection()?;
        Ok(saved_learner(&connection)?)
    }

    /// The learner while they are signed in; `None` after a log out.
    pub fn signed_in_learner(&self) -> EllaResult<Option<Learner>> {
        Ok(self
            .learner()?
            .and_then(|(learner, signed_in)| signed_in.then_some(learner)))
    }

    /// Save what the name step collects and sign the learner in, in one
    /// transaction, and hand back the row as it now stands. `created_at` and
    /// the avatar colour are left alone: they belong to the learner, not to
    /// onboarding. Somebody signed out who goes through "Let's start" again
    /// is the same learner, so their history stays theirs.
    pub fn save_learner(
        &self,
        name: &str,
        age: Option<u8>,
        level_name: &str,
        now: &str,
    ) -> EllaResult<Learner> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO learner(id, name, age, level_name, created_at)
             VALUES(1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               age = COALESCE(excluded.age, learner.age),
               level_name = excluded.level_name,
               signed_out = 0",
            params![name, age, level_name, now],
        )?;
        let (learner, _) = saved_learner(&transaction)?.ok_or_else(|| {
            EllaError::NotFound("The learner could not be read back after saving.".into())
        })?;
        transaction.commit()?;
        Ok(learner)
    }

    /// Sign the saved learner back in, exactly as they were. `None`, with
    /// nothing written, on a laptop where nobody has been saved.
    pub fn sign_in(&self) -> EllaResult<Option<Learner>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE learner SET signed_out = 0 WHERE id = 1", [])?;
        let learner = saved_learner(&transaction)?.map(|(learner, _)| learner);
        transaction.commit()?;
        Ok(learner)
    }

    /// Log out, and nothing more. The learner row, their sessions, messages,
    /// chore progress and avatar colour all stay exactly where they are.
    pub fn sign_out(&self) -> EllaResult<()> {
        self.connection()?
            .execute("UPDATE learner SET signed_out = 1 WHERE id = 1", [])?;
        Ok(())
    }

    /// Store the avatar colour and hand back the learner with it. Only while
    /// signed in: `None`, with nothing written, if the learner logged out
    /// first, so a colour picked just before a log out cannot land after it.
    pub fn save_avatar_color(&self, color: &str) -> EllaResult<Option<Learner>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE learner SET avatar_color = ?1 WHERE id = 1 AND signed_out = 0",
            params![color],
        )?;
        let learner = if changed == 0 {
            None
        } else {
            saved_learner(&transaction)?.map(|(learner, _)| learner)
        };
        transaction.commit()?;
        Ok(learner)
    }

    pub fn create_session(&self, session: &Session, opening: &Message) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO sessions(id, topic_id, topic_label, status, started_at)
             VALUES(?1, ?2, ?3, 'active', ?4)",
            params![
                session.id,
                session.topic_id,
                session.topic_label,
                session.started_at
            ],
        )?;
        insert_message(&transaction, &session.id, opening)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn session(&self, id: &str) -> EllaResult<Session> {
        let connection = self.connection()?;
        let mut session = connection
            .query_row(
                "SELECT id, topic_id, topic_label, status, started_at, completed_at
                 FROM sessions WHERE id = ?1",
                [id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        topic_id: row.get(1)?,
                        topic_label: row.get(2)?,
                        status: row.get(3)?,
                        started_at: row.get(4)?,
                        completed_at: row.get(5)?,
                        messages: Vec::new(),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| EllaError::NotFound("That conversation could not be found.".into()))?;

        let mut statement = connection.prepare(
            "SELECT id, speaker, content, turn_number, created_at
             FROM messages WHERE session_id = ?1 ORDER BY turn_number ASC, created_at ASC",
        )?;
        session.messages = statement
            .query_map([id], |row| {
                let speaker: String = row.get(1)?;
                Ok(Message {
                    id: row.get(0)?,
                    speaker: crate::domain::Speaker::from_db(&speaker),
                    content: row.get(2)?,
                    turn: row.get::<_, u32>(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(session)
    }

    pub fn recent_sessions(&self, limit: u32) -> EllaResult<Vec<SessionListItem>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT s.id, s.topic_id, s.topic_label, s.status, s.started_at, COUNT(m.id)
             FROM sessions s LEFT JOIN messages m ON m.session_id = s.id
             GROUP BY s.id ORDER BY s.started_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok(SessionListItem {
                id: row.get(0)?,
                topic_id: row.get(1)?,
                topic_label: row.get(2)?,
                status: row.get(3)?,
                started_at: row.get(4)?,
                message_count: row.get(5)?,
            })
        })?;
        let sessions = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    /// Lifetime figures over every session on the laptop, not just the five
    /// on the home screen: the streak and the badges are computed from these,
    /// and a list that stopped at five made both shrink the more the learner
    /// talked. Every session is the one learner's.
    ///
    /// An answer is a `learner` message. Days are the laptop's own calendar:
    /// timestamps are stored in UTC, and `'localtime'` moves each one onto the
    /// local day the learner said it on. A talk is counted on the day of its
    /// first answer only, so one that runs past midnight is not two talks.
    pub fn progress(&self) -> EllaResult<LearnerProgress> {
        let connection = self.connection()?;
        let mut day_statement = connection.prepare(
            "WITH answers AS (
               SELECT session_id, created_at FROM messages WHERE speaker = 'learner'
             ),
             answer_days AS (
               SELECT date(created_at, 'localtime') AS day, COUNT(*) AS answers
               FROM answers GROUP BY 1
             ),
             talk_days AS (
               SELECT day, COUNT(*) AS talks FROM (
                 SELECT date(MIN(julianday(created_at)), 'localtime') AS day
                 FROM answers GROUP BY session_id
               ) GROUP BY day
             )
             SELECT a.day, COALESCE(t.talks, 0), a.answers
             FROM answer_days a LEFT JOIN talk_days t ON t.day = a.day
             WHERE a.day IS NOT NULL
             ORDER BY a.day DESC",
        )?;
        let days = day_statement
            .query_map([], |row| {
                Ok(DayActivity {
                    day: row.get(0)?,
                    talks: row.get(1)?,
                    answers: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let answers = connection.query_row(
            "SELECT COUNT(*) FROM messages WHERE speaker = 'learner'",
            [],
            |row| row.get::<_, u32>(0),
        )?;

        // A finished talk is a completed session the learner said something
        // in. One that closed before they answered — a placement talk that
        // never heard them — earns nothing.
        let mut finished_statement = connection.prepare(
            "SELECT s.topic_id FROM sessions s
             WHERE s.status = 'complete'
               AND EXISTS (
                 SELECT 1 FROM messages m WHERE m.session_id = s.id AND m.speaker = 'learner'
               )",
        )?;
        let mut finished_topics = finished_statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let talks_finished = finished_topics.len() as u32;
        finished_topics.sort();
        finished_topics.dedup();

        Ok(LearnerProgress {
            days,
            talks_finished,
            answers,
            finished_topics,
        })
    }

    pub fn persist_turn(
        &self,
        session_id: &str,
        learner: &Message,
        ella: &Message,
    ) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        insert_message(&transaction, session_id, learner)?;
        insert_message(&transaction, session_id, ella)?;
        transaction.commit()?;
        Ok(())
    }

    /// A chore session: the same row as a free conversation plus the chore and
    /// character it belongs to, and — for ledger chores — the opening figure.
    pub fn create_chore_session(
        &self,
        session: &Session,
        opening: &Message,
        chore_id: &str,
        character_id: &str,
        ledger_opening: Option<i32>,
        now: &str,
    ) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO sessions(id, topic_id, topic_label, status, started_at, chore_id, character_id)
             VALUES(?1, ?2, ?3, 'active', ?4, ?5, ?6)",
            params![
                session.id,
                session.topic_id,
                session.topic_label,
                session.started_at,
                chore_id,
                character_id
            ],
        )?;
        if let Some(current) = ledger_opening {
            transaction.execute(
                "INSERT INTO ledger_state(session_id, current, agreed, updated_at)
                 VALUES(?1, ?2, 0, ?3)",
                params![session.id, current, now],
            )?;
        }
        insert_message(&transaction, &session.id, opening)?;
        transaction.execute(
            "INSERT INTO chore_progress(chore_id, attempts) VALUES(?1, 1)
             ON CONFLICT(chore_id) DO UPDATE SET attempts = attempts + 1",
            params![chore_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// `(chore_id, character_id)` for a session, or `None` for a free topic.
    pub fn session_chore(&self, session_id: &str) -> EllaResult<Option<(String, String)>> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT chore_id, character_id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            Some((Some(chore), Some(character))) => Some((chore, character)),
            _ => None,
        })
    }

    pub fn ledger_state(&self, session_id: &str) -> EllaResult<Option<(i32, bool)>> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT current, agreed FROM ledger_state WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get::<_, i32>(0)?, row.get::<_, i32>(1)? != 0)),
            )
            .optional()?;
        Ok(row)
    }

    pub fn save_ledger_state(
        &self,
        session_id: &str,
        current: i32,
        agreed: bool,
        now: &str,
    ) -> EllaResult<()> {
        self.connection()?.execute(
            "UPDATE ledger_state SET current = ?2, agreed = ?3, updated_at = ?4
             WHERE session_id = ?1",
            params![session_id, current, i32::from(agreed), now],
        )?;
        Ok(())
    }

    /// Written by the grader after a session ends; `passed` also stamps the
    /// chore as cleared. A failed attempt still keeps its observations.
    pub fn record_outcome(
        &self,
        session_id: &str,
        chore_id: &str,
        outcome: &str,
        best: Option<&str>,
        now: &str,
    ) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE sessions SET outcome = ?2 WHERE id = ?1",
            params![session_id, outcome],
        )?;
        if outcome == "passed" {
            transaction.execute(
                "UPDATE chore_progress SET passed_at = COALESCE(passed_at, ?2), best = ?3
                 WHERE chore_id = ?1",
                params![chore_id, now, best],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn complete_session(&self, id: &str, completed_at: &str) -> EllaResult<()> {
        let changed = self.connection()?.execute(
            "UPDATE sessions SET status = 'complete', completed_at = ?2 WHERE id = ?1",
            params![id, completed_at],
        )?;
        if changed == 0 {
            return Err(EllaError::NotFound(
                "That conversation could not be found.".into(),
            ));
        }
        Ok(())
    }

    /// Fold the write-ahead log back into `ella.sqlite3` and empty it.
    ///
    /// Every commit is already safe in `ella.sqlite3-wal`. While the app runs,
    /// SQLite folds the log in by itself only once it passes about 1000 pages
    /// (some 4 MB), so the newest talks can live only in the log, and a young
    /// database can be almost empty beside it. The app exits without closing
    /// this connection, so SQLite's own fold-in on close never happens either.
    /// Doing it on quit means `ella.sqlite3` alone is the learner's whole
    /// history, which is what somebody copying it to another laptop expects.
    /// Harmless to repeat: a second call finds nothing to move.
    pub fn checkpoint(&self) -> EllaResult<()> {
        let connection = self.connection()?;
        let busy = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            row.get::<_, i64>(0)
        })?;
        if busy != 0 {
            return Err(EllaError::Conflict(
                "The database was still in use, so its write-ahead log was not folded in.".into(),
            ));
        }
        Ok(())
    }
}

/// The one learner row and whether it is signed in. Shared by the reads and
/// by the writes that hand the row back from inside their own transaction.
fn saved_learner(connection: &Connection) -> rusqlite::Result<Option<(Learner, bool)>> {
    connection
        .query_row(
            "SELECT name, age, level_name, created_at, avatar_color, signed_out = 0
             FROM learner WHERE id = 1",
            [],
            |row| Ok((learner_from_row(row)?, row.get::<_, bool>(5)?)),
        )
        .optional()
}

fn learner_from_row(row: &Row<'_>) -> rusqlite::Result<Learner> {
    Ok(Learner {
        name: row.get(0)?,
        age: row.get(1)?,
        level_name: row.get(2)?,
        created_at: row.get(3)?,
        avatar_color: row.get(4)?,
    })
}

fn has_column(connection: &Connection, table: &str, column: &str) -> EllaResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |row| row.get(0),
    )?)
}

fn has_table(connection: &Connection, table: &str) -> EllaResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get(0),
    )?)
}

/// Whether the learner table still carries the `CHECK (id = 1)` that every
/// release has made it with, which is what keeps the laptop to one learner.
fn learner_is_pinned(connection: &Connection) -> EllaResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master
           WHERE type = 'table' AND name = 'learner'
             AND replace(sql, ' ', '') LIKE '%CHECK(id=1)%'
         )",
        [],
        |row| row.get(0),
    )?)
}

/// Databases written before the Zoe -> Ella rename store `speaker = 'zoe'` and
/// carry a CHECK constraint that rejects `'ella'`. SQLite cannot alter a CHECK
/// in place, so rebuild the table once and copy the rows across.
fn rename_legacy_speaker(connection: &Connection) -> EllaResult<()> {
    let legacy: bool = connection.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master
           WHERE type = 'table' AND name = 'messages' AND sql LIKE '%''zoe''%'
         )",
        [],
        |row| row.get(0),
    )?;
    if !legacy {
        return Ok(());
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = OFF;
         BEGIN IMMEDIATE;
         CREATE TABLE messages_migrated (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           speaker TEXT NOT NULL CHECK (speaker IN ('learner', 'ella')),
           content TEXT NOT NULL,
           turn_number INTEGER NOT NULL,
           created_at TEXT NOT NULL
         );
         INSERT INTO messages_migrated(id, session_id, speaker, content, turn_number, created_at)
           SELECT id, session_id,
                  CASE speaker WHEN 'zoe' THEN 'ella' ELSE speaker END,
                  content, turn_number, created_at
           FROM messages;
         DROP TABLE messages;
         ALTER TABLE messages_migrated RENAME TO messages;
         CREATE INDEX IF NOT EXISTS idx_messages_session_turn
           ON messages(session_id, turn_number, created_at);
         COMMIT;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

/// `age` arrived with the v6 onboarding flow; older databases predate the column.
fn add_learner_age(connection: &Connection) -> EllaResult<()> {
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('learner') WHERE name = 'age')",
        [],
        |row| row.get(0),
    )?;
    if !present {
        connection.execute_batch("ALTER TABLE learner ADD COLUMN age INTEGER;")?;
    }
    Ok(())
}

fn insert_message(
    transaction: &Transaction<'_>,
    session_id: &str,
    message: &Message,
) -> EllaResult<()> {
    transaction.execute(
        "INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            message.id,
            session_id,
            message.speaker.as_str(),
            message.content,
            message.turn,
            message.created_at
        ],
    )?;
    Ok(())
}

fn widen_message_speaker(connection: &Connection) -> EllaResult<()> {
    let narrow: bool = connection.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master
           WHERE type = 'table' AND name = 'messages' AND sql LIKE '%''ella''%'
         )",
        [],
        |row| row.get(0),
    )?;
    if !narrow {
        return Ok(());
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = OFF;
         BEGIN IMMEDIATE;
         CREATE TABLE messages_widened (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           speaker TEXT NOT NULL CHECK (length(speaker) > 0),
           content TEXT NOT NULL,
           turn_number INTEGER NOT NULL,
           created_at TEXT NOT NULL
         );
         INSERT INTO messages_widened(id, session_id, speaker, content, turn_number, created_at)
           SELECT id, session_id, speaker, content, turn_number, created_at FROM messages;
         DROP TABLE messages;
         ALTER TABLE messages_widened RENAME TO messages;
         CREATE INDEX IF NOT EXISTS idx_messages_session_turn
           ON messages(session_id, turn_number, created_at);
         COMMIT;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

/// Chore, ledger, observation and character state. Additive: `skill_progress`
/// is left in place for one release of overlap rather than dropped here.
fn add_chore_tables(connection: &Connection) -> EllaResult<()> {
    for (table, column, ddl) in [
        ("learner", "name_spoken", "ALTER TABLE learner ADD COLUMN name_spoken TEXT"),
        ("learner", "interests", "ALTER TABLE learner ADD COLUMN interests TEXT"),
        ("sessions", "chore_id", "ALTER TABLE sessions ADD COLUMN chore_id TEXT"),
        ("sessions", "character_id", "ALTER TABLE sessions ADD COLUMN character_id TEXT"),
        ("sessions", "outcome", "ALTER TABLE sessions ADD COLUMN outcome TEXT"),
    ] {
        let present: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
            params![table, column],
            |row| row.get(0),
        )?;
        if !present {
            connection.execute(ddl, [])?;
        }
    }
    // `outcome` cannot carry a CHECK because it arrives via ALTER; the enum is
    // enforced in Rust where the value is written.
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS ledger_state (
           session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
           current INTEGER NOT NULL,
           agreed INTEGER NOT NULL DEFAULT 0,
           updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS observations (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           kind TEXT NOT NULL CHECK (kind IN ('grammar','vocabulary','fluency','pronunciation')),
           tag TEXT NOT NULL,
           said TEXT NOT NULL,
           better TEXT,
           meant TEXT,
           confirmed_by TEXT,
           addressed_in TEXT REFERENCES sessions(id),
           created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_observations_open ON observations(tag, addressed_in);
         CREATE TABLE IF NOT EXISTS chore_progress (
           chore_id TEXT PRIMARY KEY,
           attempts INTEGER NOT NULL DEFAULT 0,
           passed_at TEXT,
           best TEXT
         );
         CREATE TABLE IF NOT EXISTS character_state (
           character_id TEXT PRIMARY KEY,
           turns_talked INTEGER NOT NULL DEFAULT 0,
           last_hook TEXT,
           last_met_at TEXT,
           memory TEXT
         );",
    )?;
    Ok(())
}

/// Two things the learner row has gained since v0.1.6, added the way `age`
/// and `name_spoken` were, so v0.1.6's own statements still work on it: the
/// avatar colour, which used to live only in the window, and whether the
/// learner has logged out. "Log out" used to delete the learner and every
/// talk they had had; now it only sets `signed_out`.
fn add_learner_profile(connection: &Connection) -> EllaResult<()> {
    for (column, ddl) in [
        ("avatar_color", "ALTER TABLE learner ADD COLUMN avatar_color TEXT"),
        (
            "signed_out",
            "ALTER TABLE learner ADD COLUMN signed_out INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !has_column(connection, "learner", column)? {
            connection.execute(ddl, [])?;
        }
    }
    Ok(())
}

/// For a few hours of development, and never in a release, the laptop could
/// hold several learners: the learner table lost its `CHECK (id = 1)`, a
/// `device_state` row named whoever was signed in, sessions carried a
/// `learner_id`, and chore and character state were keyed by learner too.
/// There is one learner per laptop, so a database a development build left in
/// that layout is folded back, once, into the one v0.1.6 reads and writes.
///
/// The learner who stays, as id 1, is whoever was signed in; with nobody
/// signed in it is whoever was active last, and they stay logged out, as
/// they were. Every session and message stays, whoever's it was: the
/// laptop's history is the one learner's. Chore attempts are added up per
/// chore, keeping the earliest pass and the highest `best`.
/// `sessions.learner_id` is emptied rather than dropped, because SQLite will
/// not drop a column that carries a REFERENCES clause; nothing reads it.
///
/// All of it happens in one transaction, so a crash part-way leaves the old
/// layout whole and the next launch starts again from the top. Each step
/// checks for itself what is left to do, which also covers a database whose
/// move into that layout was itself cut short. A `device_state` table is
/// taken as a sign of that layout and dropped, so the name is not free for
/// anything new.
fn keep_one_learner(connection: &Connection) -> EllaResult<()> {
    let several = !learner_is_pinned(connection)?
        || has_table(connection, "device_state")?
        || has_column(connection, "chore_progress", "learner_id")?
        || has_column(connection, "character_state", "learner_id")?;
    if !several {
        return Ok(());
    }
    // As in the other rebuilds here, foreign keys go off outside the
    // transaction, the only place SQLite honours the pragma, so dropping the
    // old tables does not touch the rows that point at them.
    connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let folded = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(EllaError::from)
        .and_then(|transaction| {
            fold_into_one_learner(&transaction)?;
            transaction.commit()?;
            Ok(())
        });
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    folded
}

/// The steps of `keep_one_learner`, run inside its transaction.
fn fold_into_one_learner(connection: &Connection) -> EllaResult<()> {
    if !learner_is_pinned(connection)? {
        let signed_in = if has_table(connection, "device_state")? {
            connection
                .query_row(
                    "SELECT l.id FROM device_state d JOIN learner l ON l.id = CAST(d.value AS INTEGER)
                     WHERE d.key = 'signed_in_learner'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
        } else {
            None
        };
        // Both learner tables the development build made, new and upgraded,
        // have every column read here.
        connection.execute_batch(
            "CREATE TABLE learner_single (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               name TEXT NOT NULL,
               age INTEGER,
               level_name TEXT NOT NULL,
               created_at TEXT NOT NULL,
               name_spoken TEXT,
               interests TEXT,
               avatar_color TEXT,
               signed_out INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        connection.execute(
            "INSERT INTO learner_single(
               id, name, age, level_name, created_at, name_spoken, interests, avatar_color, signed_out
             )
               SELECT 1, name, age, level_name, created_at, name_spoken, interests, avatar_color, ?2
               FROM learner
               WHERE id = COALESCE(
                 ?1,
                 (SELECT id FROM learner ORDER BY last_active_at DESC, id DESC LIMIT 1)
               )",
            params![signed_in, signed_in.is_none()],
        )?;
        connection.execute_batch(
            "DROP TABLE learner;
             ALTER TABLE learner_single RENAME TO learner;",
        )?;
    }
    if has_column(connection, "chore_progress", "learner_id")? {
        connection.execute_batch(
            "CREATE TABLE chore_progress_by_chore (
               chore_id TEXT PRIMARY KEY,
               attempts INTEGER NOT NULL DEFAULT 0,
               passed_at TEXT,
               best TEXT
             );
             INSERT INTO chore_progress_by_chore(chore_id, attempts, passed_at, best)
               SELECT chore_id, SUM(attempts), MIN(passed_at), MAX(best)
               FROM chore_progress GROUP BY chore_id;
             DROP TABLE chore_progress;
             ALTER TABLE chore_progress_by_chore RENAME TO chore_progress;",
        )?;
    }
    if has_column(connection, "character_state", "learner_id")? {
        connection.execute_batch(
            "CREATE TABLE character_state_by_character (
               character_id TEXT PRIMARY KEY,
               turns_talked INTEGER NOT NULL DEFAULT 0,
               last_hook TEXT,
               last_met_at TEXT,
               memory TEXT
             );
             INSERT INTO character_state_by_character(
               character_id, turns_talked, last_hook, last_met_at, memory
             )
               SELECT character_id, MAX(turns_talked), MAX(last_hook), MAX(last_met_at), MAX(memory)
               FROM character_state GROUP BY character_id;
             DROP TABLE character_state;
             ALTER TABLE character_state_by_character RENAME TO character_state;",
        )?;
    }
    if has_column(connection, "sessions", "learner_id")? {
        connection.execute(
            "UPDATE sessions SET learner_id = NULL WHERE learner_id IS NOT NULL",
            [],
        )?;
    }
    connection.execute_batch(
        "DROP INDEX IF EXISTS idx_sessions_learner;
         DROP TABLE IF EXISTS device_state;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Speaker;
    use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
    use rusqlite::types::Value;
    use std::fs;
    use uuid::Uuid;

    /// v0.1.6's base schema, run on every launch of that release.
    const V0_1_6_BASE: &str = "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS learner (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               name TEXT NOT NULL,
               age INTEGER,
               level_name TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               topic_id TEXT NOT NULL,
               topic_label TEXT NOT NULL,
               status TEXT NOT NULL CHECK (status IN ('active', 'complete')),
               started_at TEXT NOT NULL,
               completed_at TEXT
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               speaker TEXT NOT NULL CHECK (speaker IN ('learner', 'ella')),
               content TEXT NOT NULL,
               turn_number INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_session_turn
               ON messages(session_id, turn_number, created_at);
";

    /// v0.1.6's speaker widening, which also ran unchanged in the
    /// multi-learner development build.
    const WIDEN_MESSAGE_SPEAKER: &str = "PRAGMA foreign_keys = OFF;
         BEGIN IMMEDIATE;
         CREATE TABLE messages_widened (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           speaker TEXT NOT NULL CHECK (length(speaker) > 0),
           content TEXT NOT NULL,
           turn_number INTEGER NOT NULL,
           created_at TEXT NOT NULL
         );
         INSERT INTO messages_widened(id, session_id, speaker, content, turn_number, created_at)
           SELECT id, session_id, speaker, content, turn_number, created_at FROM messages;
         DROP TABLE messages;
         ALTER TABLE messages_widened RENAME TO messages;
         CREATE INDEX IF NOT EXISTS idx_messages_session_turn
           ON messages(session_id, turn_number, created_at);
         COMMIT;
         PRAGMA foreign_keys = ON;";

    const SESSION_CHORE_COLUMNS: [&str; 3] = [
        "ALTER TABLE sessions ADD COLUMN chore_id TEXT",
        "ALTER TABLE sessions ADD COLUMN character_id TEXT",
        "ALTER TABLE sessions ADD COLUMN outcome TEXT",
    ];

    /// What a v0.1.6 install ran against a fresh file, statement for
    /// statement: its base schema, the speaker widening, then the chore
    /// columns and tables. Copied from that release so the migrations are
    /// tested against the shape every existing install really has on disk.
    fn create_v0_1_6_schema(connection: &Connection) {
        connection.execute_batch(V0_1_6_BASE).unwrap();
        connection.execute_batch(WIDEN_MESSAGE_SPEAKER).unwrap();
        for ddl in [
            "ALTER TABLE learner ADD COLUMN name_spoken TEXT",
            "ALTER TABLE learner ADD COLUMN interests TEXT",
        ]
        .into_iter()
        .chain(SESSION_CHORE_COLUMNS)
        {
            connection.execute(ddl, []).unwrap();
        }
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ledger_state (
           session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
           current INTEGER NOT NULL,
           agreed INTEGER NOT NULL DEFAULT 0,
           updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS observations (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           kind TEXT NOT NULL CHECK (kind IN ('grammar','vocabulary','fluency','pronunciation')),
           tag TEXT NOT NULL,
           said TEXT NOT NULL,
           better TEXT,
           meant TEXT,
           confirmed_by TEXT,
           addressed_in TEXT REFERENCES sessions(id),
           created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_observations_open ON observations(tag, addressed_in);
         CREATE TABLE IF NOT EXISTS chore_progress (
           chore_id TEXT PRIMARY KEY,
           attempts INTEGER NOT NULL DEFAULT 0,
           passed_at TEXT,
           best TEXT
         );
         CREATE TABLE IF NOT EXISTS character_state (
           character_id TEXT PRIMARY KEY,
           turns_talked INTEGER NOT NULL DEFAULT 0,
           last_hook TEXT,
           last_met_at TEXT,
           memory TEXT
         );",
            )
            .unwrap();
    }

    /// The very first builds: a Zoe-era speaker CHECK, no age column, and the
    /// garden's `skill_progress` table.
    fn create_first_release_schema(connection: &Connection) {
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS learner (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               name TEXT NOT NULL,
               level_name TEXT NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               topic_id TEXT NOT NULL,
               topic_label TEXT NOT NULL,
               status TEXT NOT NULL CHECK (status IN ('active', 'complete')),
               started_at TEXT NOT NULL,
               completed_at TEXT
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               speaker TEXT NOT NULL CHECK (speaker IN ('learner', 'zoe')),
               content TEXT NOT NULL,
               turn_number INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_session_turn
               ON messages(session_id, turn_number, created_at);
             CREATE TABLE IF NOT EXISTS skill_progress (
               skill_id TEXT PRIMARY KEY,
               label TEXT NOT NULL,
               strand TEXT NOT NULL,
               evidence_count INTEGER NOT NULL DEFAULT 0,
               last_evidence TEXT,
               updated_at TEXT NOT NULL
             );",
            )
            .unwrap();
    }

    /// The multi-learner development build's base schema, copied from it:
    /// learners without `CHECK (id = 1)`, and the signed-in pointer table.
    const MULTI_BASE: &str = "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             CREATE TABLE IF NOT EXISTS learner (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               age INTEGER,
               level_name TEXT NOT NULL,
               created_at TEXT NOT NULL,
               name_spoken TEXT,
               interests TEXT,
               avatar_color TEXT,
               last_active_at TEXT
             );
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               topic_id TEXT NOT NULL,
               topic_label TEXT NOT NULL,
               status TEXT NOT NULL CHECK (status IN ('active', 'complete')),
               started_at TEXT NOT NULL,
               completed_at TEXT
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               speaker TEXT NOT NULL CHECK (speaker IN ('learner', 'ella')),
               content TEXT NOT NULL,
               turn_number INTEGER NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_session_turn
               ON messages(session_id, turn_number, created_at);
             CREATE TABLE IF NOT EXISTS device_state (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
";

    /// The development build's chore tables, keyed by learner.
    const MULTI_CHORE_TABLES: &str = "CREATE TABLE IF NOT EXISTS ledger_state (
           session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
           current INTEGER NOT NULL,
           agreed INTEGER NOT NULL DEFAULT 0,
           updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS observations (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           kind TEXT NOT NULL CHECK (kind IN ('grammar','vocabulary','fluency','pronunciation')),
           tag TEXT NOT NULL,
           said TEXT NOT NULL,
           better TEXT,
           meant TEXT,
           confirmed_by TEXT,
           addressed_in TEXT REFERENCES sessions(id),
           created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_observations_open ON observations(tag, addressed_in);
         CREATE TABLE IF NOT EXISTS chore_progress (
           learner_id INTEGER NOT NULL REFERENCES learner(id),
           chore_id TEXT NOT NULL,
           attempts INTEGER NOT NULL DEFAULT 0,
           passed_at TEXT,
           best TEXT,
           PRIMARY KEY (learner_id, chore_id)
         );
         CREATE TABLE IF NOT EXISTS character_state (
           learner_id INTEGER NOT NULL REFERENCES learner(id),
           character_id TEXT NOT NULL,
           turns_talked INTEGER NOT NULL DEFAULT 0,
           last_hook TEXT,
           last_met_at TEXT,
           memory TEXT,
           PRIMARY KEY (learner_id, character_id)
         );";

    /// The development build's `allow_many_learners`, which rebuilt a v0.1.6
    /// learner table without its CHECK and signed its learner in.
    const MULTI_ALLOW_MANY_LEARNERS: &str = "PRAGMA foreign_keys = OFF;
         BEGIN IMMEDIATE;
         CREATE TABLE learner_many (
           id INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           age INTEGER,
           level_name TEXT NOT NULL,
           created_at TEXT NOT NULL,
           name_spoken TEXT,
           interests TEXT,
           avatar_color TEXT,
           last_active_at TEXT
         );
         INSERT INTO learner_many(id, name, age, level_name, created_at, name_spoken, interests, last_active_at)
           SELECT id, name, age, level_name, created_at, name_spoken, interests,
                  COALESCE((SELECT MAX(started_at) FROM sessions), created_at)
           FROM learner;
         DROP TABLE learner;
         ALTER TABLE learner_many RENAME TO learner;
         INSERT OR IGNORE INTO device_state(key, value)
           SELECT 'signed_in_learner', CAST(id AS TEXT) FROM learner ORDER BY id LIMIT 1;
         COMMIT;
         PRAGMA foreign_keys = ON;";

    /// The development build's `add_session_learner`.
    const MULTI_ADD_SESSION_LEARNER: [&str; 2] = [
        "BEGIN IMMEDIATE;
             ALTER TABLE sessions ADD COLUMN learner_id INTEGER REFERENCES learner(id);
             UPDATE sessions SET learner_id = (SELECT MIN(id) FROM learner);
             COMMIT;",
        "CREATE INDEX IF NOT EXISTS idx_sessions_learner ON sessions(learner_id, started_at);",
    ];

    /// The development build's `key_chore_state_by_learner`.
    const MULTI_KEY_CHORE_STATE_BY_LEARNER: [&str; 2] = [
        "PRAGMA foreign_keys = OFF;
             BEGIN IMMEDIATE;
             CREATE TABLE chore_progress_by_learner (
               learner_id INTEGER NOT NULL REFERENCES learner(id),
               chore_id TEXT NOT NULL,
               attempts INTEGER NOT NULL DEFAULT 0,
               passed_at TEXT,
               best TEXT,
               PRIMARY KEY (learner_id, chore_id)
             );
             INSERT INTO chore_progress_by_learner(learner_id, chore_id, attempts, passed_at, best)
               SELECT (SELECT MIN(id) FROM learner), chore_id, attempts, passed_at, best
               FROM chore_progress WHERE EXISTS (SELECT 1 FROM learner);
             DROP TABLE chore_progress;
             ALTER TABLE chore_progress_by_learner RENAME TO chore_progress;
             COMMIT;
             PRAGMA foreign_keys = ON;",
        "PRAGMA foreign_keys = OFF;
             BEGIN IMMEDIATE;
             CREATE TABLE character_state_by_learner (
               learner_id INTEGER NOT NULL REFERENCES learner(id),
               character_id TEXT NOT NULL,
               turns_talked INTEGER NOT NULL DEFAULT 0,
               last_hook TEXT,
               last_met_at TEXT,
               memory TEXT,
               PRIMARY KEY (learner_id, character_id)
             );
             INSERT INTO character_state_by_learner(
               learner_id, character_id, turns_talked, last_hook, last_met_at, memory
             )
               SELECT (SELECT MIN(id) FROM learner),
                      character_id, turns_talked, last_hook, last_met_at, memory
               FROM character_state WHERE EXISTS (SELECT 1 FROM learner);
             DROP TABLE character_state;
             ALTER TABLE character_state_by_learner RENAME TO character_state;
             COMMIT;
             PRAGMA foreign_keys = ON;",
    ];

    /// What the multi-learner development build ran against a fresh file, in
    /// its order, so the fold back is tested against the layout it really left.
    fn create_intermediate_schema(connection: &Connection) {
        connection.execute_batch(MULTI_BASE).unwrap();
        connection.execute_batch(WIDEN_MESSAGE_SPEAKER).unwrap();
        for ddl in SESSION_CHORE_COLUMNS {
            connection.execute(ddl, []).unwrap();
        }
        connection.execute_batch(MULTI_CHORE_TABLES).unwrap();
        for batch in MULTI_ADD_SESSION_LEARNER {
            connection.execute_batch(batch).unwrap();
        }
    }

    /// What the development build did to a v0.1.6 laptop when it opened it,
    /// which is how the one developer database in that layout got there.
    fn upgrade_v0_1_6_to_intermediate(connection: &Connection) {
        connection.execute_batch(MULTI_BASE).unwrap();
        connection.execute_batch(MULTI_CHORE_TABLES).unwrap();
        connection.execute_batch(MULTI_ALLOW_MANY_LEARNERS).unwrap();
        for batch in MULTI_ADD_SESSION_LEARNER
            .into_iter()
            .chain(MULTI_KEY_CHORE_STATE_BY_LEARNER)
        {
            connection.execute_batch(batch).unwrap();
        }
    }

    /// A tester's v0.1.6 laptop: one learner, two free talks (one finished,
    /// one left open), a finished chore with its ledger, and the chore and
    /// character state that went with it.
    const V0_1_6_LEARNER_DATA: &str = "
        INSERT INTO learner(id, name, age, level_name, created_at)
          VALUES (1, 'Meera', 15, 'Morning Meadow', '2026-09-01T08:00:00+00:00');
        INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at)
          VALUES ('s1', 'street-food', 'Street food stories', 'complete',
                  '2026-09-02T12:00:00+00:00', '2026-09-02T12:10:00+00:00'),
                 ('s2', 'restaurant-order', 'Ordering at a restaurant', 'active',
                  '2026-09-03T12:00:00+00:00', NULL);
        INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at,
                             chore_id, character_id, outcome)
          VALUES ('s3', 'market-cloth-price', 'Talk a stall price down', 'complete',
                  '2026-09-04T12:00:00+00:00', '2026-09-04T12:20:00+00:00',
                  'market-cloth-price', 'stall-owner', 'passed');
        INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at) VALUES
          ('m1', 's1', 'ella', 'What did you eat today?', 0, '2026-09-02T12:00:00+00:00'),
          ('m2', 's1', 'learner', 'I ate poha', 1, '2026-09-02T12:01:00+00:00'),
          ('m3', 's1', 'ella', 'Was it spicy?', 1, '2026-09-02T12:01:00+00:00'),
          ('m4', 's1', 'learner', 'A little spicy', 2, '2026-09-02T12:02:00+00:00'),
          ('m5', 's1', 'ella', 'Lovely.', 2, '2026-09-02T12:02:00+00:00'),
          ('m6', 's2', 'ella', 'What would you like?', 0, '2026-09-03T12:00:00+00:00'),
          ('m7', 's2', 'learner', 'One dosa please', 1, '2026-09-03T12:01:00+00:00'),
          ('m8', 's2', 'ella', 'Anything to drink?', 1, '2026-09-03T12:01:00+00:00'),
          ('m9', 's3', 'stall-owner', 'Six hundred rupees.', 0, '2026-09-04T12:00:00+00:00'),
          ('m10', 's3', 'learner', 'Four hundred?', 1, '2026-09-04T12:01:00+00:00'),
          ('m11', 's3', 'stall-owner', 'Alright, four hundred.', 1, '2026-09-04T12:01:00+00:00');
        INSERT INTO ledger_state(session_id, current, agreed, updated_at)
          VALUES ('s3', 400, 1, '2026-09-04T12:01:00+00:00');
        INSERT INTO observations(id, session_id, kind, tag, said, better, created_at)
          VALUES ('o1', 's3', 'grammar', 'question-form', 'Four hundred?',
                  'Could you do four hundred?', '2026-09-04T12:20:00+00:00');
        INSERT INTO chore_progress(chore_id, attempts, passed_at, best)
          VALUES ('market-cloth-price', 2, '2026-09-04T12:20:00+00:00', '400');
        INSERT INTO character_state(character_id, turns_talked, last_hook, last_met_at, memory)
          VALUES ('stall-owner', 5, 'the blue shirt', '2026-09-04T12:20:00+00:00', NULL);
    ";

    /// Two learners on a development build: Asha, active most recently, and
    /// Ravi. Each has free talks and a go at the same chore, and Ravi has a
    /// second chore of his own. Whether and to whom `device_state` points is
    /// left to each test.
    const INTERMEDIATE_DATA: &str = "
        INSERT INTO learner(id, name, age, level_name, created_at, name_spoken, interests,
                            avatar_color, last_active_at) VALUES
          (1, 'Asha', 14, 'Morning Meadow', '2026-09-20T08:00:00+00:00', 'AH-sha', 'cricket',
           '#7C5CFF', '2026-09-25T10:00:00+00:00'),
          (2, 'Ravi', 20, 'Morning Meadow', '2026-09-21T08:00:00+00:00', NULL, 'films',
           '#FF8800', '2026-09-24T10:00:00+00:00');
        INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at, learner_id)
          VALUES ('a1', 'street-food', 'Street food stories', 'complete',
                  '2026-09-20T12:00:00+00:00', '2026-09-20T12:10:00+00:00', 1),
                 ('r1', 'booking-a-cab', 'Booking a cab', 'complete',
                  '2026-09-21T12:00:00+00:00', '2026-09-21T12:10:00+00:00', 2),
                 ('r2', 'restaurant-order', 'Ordering at a restaurant', 'active',
                  '2026-09-22T12:00:00+00:00', NULL, 2);
        INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at,
                             chore_id, character_id, outcome, learner_id)
          VALUES ('a2', 'market-cloth-price', 'Talk a stall price down', 'complete',
                  '2026-09-23T12:00:00+00:00', '2026-09-23T12:20:00+00:00',
                  'market-cloth-price', 'stall-owner', 'passed', 1),
                 ('r3', 'market-cloth-price', 'Talk a stall price down', 'complete',
                  '2026-09-24T12:00:00+00:00', '2026-09-24T12:20:00+00:00',
                  'market-cloth-price', 'stall-owner', 'passed', 2);
        INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at) VALUES
          ('m1', 'a1', 'ella', 'What did you eat today?', 0, '2026-09-20T12:00:00+00:00'),
          ('m2', 'a1', 'learner', 'I ate poha', 1, '2026-09-20T12:01:00+00:00'),
          ('m3', 'a1', 'ella', 'Was it spicy?', 1, '2026-09-20T12:01:00+00:00'),
          ('m4', 'a1', 'learner', 'A little spicy', 2, '2026-09-20T12:02:00+00:00'),
          ('m5', 'a1', 'ella', 'Lovely.', 2, '2026-09-20T12:02:00+00:00'),
          ('m6', 'r1', 'ella', 'Where are you going?', 0, '2026-09-21T12:00:00+00:00'),
          ('m7', 'r1', 'learner', 'To the station', 1, '2026-09-21T12:01:00+00:00'),
          ('m8', 'r1', 'ella', 'Right away.', 1, '2026-09-21T12:01:00+00:00'),
          ('m9', 'r2', 'ella', 'What would you like?', 0, '2026-09-22T12:00:00+00:00'),
          ('m10', 'r2', 'learner', 'One dosa please', 1, '2026-09-22T12:01:00+00:00'),
          ('m11', 'r2', 'ella', 'Anything to drink?', 1, '2026-09-22T12:01:00+00:00'),
          ('m12', 'a2', 'stall-owner', 'Six hundred rupees.', 0, '2026-09-23T12:00:00+00:00'),
          ('m13', 'a2', 'learner', 'Four hundred?', 1, '2026-09-23T12:01:00+00:00'),
          ('m14', 'a2', 'stall-owner', 'Alright, four hundred.', 1, '2026-09-23T12:01:00+00:00'),
          ('m15', 'r3', 'stall-owner', 'Six hundred rupees.', 0, '2026-09-24T12:00:00+00:00');
        INSERT INTO ledger_state(session_id, current, agreed, updated_at) VALUES
          ('a2', 400, 1, '2026-09-23T12:01:00+00:00'),
          ('r3', 600, 0, '2026-09-24T12:00:00+00:00');
        INSERT INTO observations(id, session_id, kind, tag, said, better, created_at)
          VALUES ('o1', 'a2', 'grammar', 'question-form', 'Four hundred?',
                  'Could you do four hundred?', '2026-09-23T12:20:00+00:00');
        INSERT INTO chore_progress(learner_id, chore_id, attempts, passed_at, best) VALUES
          (1, 'market-cloth-price', 2, '2026-09-23T12:20:00+00:00', '400'),
          (2, 'market-cloth-price', 1, '2026-09-24T12:20:00+00:00', '380'),
          (2, 'metro-card-topup', 1, NULL, NULL);
        INSERT INTO character_state(learner_id, character_id, turns_talked, last_hook,
                                    last_met_at, memory) VALUES
          (1, 'stall-owner', 5, 'the blue shirt', '2026-09-23T12:20:00+00:00', NULL),
          (2, 'stall-owner', 3, 'the red scarf', '2026-09-24T12:20:00+00:00', 'haggles hard');
    ";

    const V0_1_6_TABLES: [&str; 7] = [
        "learner",
        "sessions",
        "messages",
        "ledger_state",
        "observations",
        "chore_progress",
        "character_state",
    ];

    fn count(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    /// Every row a query returns, each written out whole.
    fn rows(connection: &Connection, sql: &str) -> Vec<String> {
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                Ok((0..columns)
                    .map(|index| format!("{:?}", row.get::<_, Value>(index).unwrap()))
                    .collect::<Vec<_>>()
                    .join(" | "))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    /// Every table, index and definition on file, in a stable order.
    fn schema(connection: &Connection) -> Vec<String> {
        rows(
            connection,
            "SELECT type, name, sql FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
    }

    /// Every table's definition and every row in it, in a stable order, so two
    /// launches can be compared whole.
    fn dump(database: &Database) -> Vec<String> {
        let connection = database.connection().unwrap();
        let mut lines = schema(&connection);
        let tables = rows(
            &connection,
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        );
        for table in tables {
            // `rows` writes each name out as `Text("...")`.
            let table = table.trim_start_matches("Text(\"").trim_end_matches("\")");
            lines.extend(
                rows(&connection, &format!("SELECT * FROM {table} ORDER BY rowid"))
                    .into_iter()
                    .map(|row| format!("{table}: {row}")),
            );
        }
        lines
    }

    /// Both of SQLite's own checks, which a rebuild that dropped or mangled
    /// anything would fail.
    fn assert_sound(connection: &Connection) {
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert!(
            rows(connection, "PRAGMA foreign_key_check").is_empty(),
            "no row points at something that is not there"
        );
    }

    /// v0.1.6's own statements, copied from that release, run against a
    /// database this version has opened: somebody who goes back to v0.1.6
    /// finds a laptop it can still read and write. Its `reset` is left out;
    /// nothing here still has one.
    fn use_as_v0_1_6(path: &Path) {
        let mut connection = Connection::open(path).unwrap();
        // Its launch: the base schema, then checks that all find their
        // columns already there.
        connection.execute_batch(V0_1_6_BASE).unwrap();
        let sessions_before = count(&connection, "sessions");
        let attempts_before: i64 = connection
            .query_row(
                "SELECT COALESCE(SUM(attempts), 0) FROM chore_progress
                 WHERE chore_id = 'market-cloth-price'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // `learner()` and `save_learner()`, run again without the age step.
        let learner = |connection: &Connection| {
            connection
                .query_row(
                    "SELECT name, age, level_name, created_at FROM learner WHERE id = 1",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<u8>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .unwrap()
        };
        let (name, age, _, created_at) = learner(&connection);
        connection
            .execute(
                "INSERT INTO learner(id, name, age, level_name, created_at)
             VALUES(1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               age = COALESCE(excluded.age, learner.age),
               level_name = excluded.level_name",
                params![
                    format!("{name} again"),
                    Option::<u8>::None,
                    "Morning Meadow",
                    "2026-10-01T08:00:00+00:00"
                ],
            )
            .unwrap();
        assert_eq!(
            learner(&connection),
            (format!("{name} again"), age, "Morning Meadow".into(), created_at)
        );

        // `create_session`, `persist_turn` and `complete_session`.
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO sessions(id, topic_id, topic_label, status, started_at)
             VALUES(?1, ?2, ?3, 'active', ?4)",
                params!["v016-free", "street-food", "Street food stories", "2026-10-01T12:00:00+00:00"],
            )
            .unwrap();
        for (id, speaker, content, turn, at) in [
            ("v016-m1", "ella", "What did you eat?", 0, "2026-10-01T12:00:00+00:00"),
            ("v016-m2", "learner", "Idli", 1, "2026-10-01T12:01:00+00:00"),
            ("v016-m3", "ella", "Tasty!", 1, "2026-10-01T12:01:00+00:00"),
        ] {
            transaction
                .execute(
                    "INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, "v016-free", speaker, content, turn, at],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        connection
            .execute(
                "UPDATE sessions SET status = 'complete', completed_at = ?2 WHERE id = ?1",
                params!["v016-free", "2026-10-01T12:05:00+00:00"],
            )
            .unwrap();

        // `create_chore_session` and `record_outcome`.
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO sessions(id, topic_id, topic_label, status, started_at, chore_id, character_id)
             VALUES(?1, ?2, ?3, 'active', ?4, ?5, ?6)",
                params![
                    "v016-chore",
                    "market-cloth-price",
                    "Talk a stall price down",
                    "2026-10-02T12:00:00+00:00",
                    "market-cloth-price",
                    "stall-owner"
                ],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO ledger_state(session_id, current, agreed, updated_at)
                 VALUES(?1, ?2, 0, ?3)",
                params!["v016-chore", 600, "2026-10-02T12:00:00+00:00"],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "v016-m4",
                    "v016-chore",
                    "stall-owner",
                    "Six hundred.",
                    0,
                    "2026-10-02T12:00:00+00:00"
                ],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO chore_progress(chore_id, attempts) VALUES(?1, 1)
             ON CONFLICT(chore_id) DO UPDATE SET attempts = attempts + 1",
                params!["market-cloth-price"],
            )
            .unwrap();
        transaction.commit().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "UPDATE sessions SET outcome = ?2 WHERE id = ?1",
                params!["v016-chore", "passed"],
            )
            .unwrap();
        transaction
            .execute(
                "UPDATE chore_progress SET passed_at = COALESCE(passed_at, ?2), best = ?3
                 WHERE chore_id = ?1",
                params!["market-cloth-price", "2026-10-02T12:20:00+00:00", "350"],
            )
            .unwrap();
        transaction.commit().unwrap();

        // `recent_sessions(5)` and `completed_conversations()`.
        let recent = rows(
            &connection,
            "SELECT s.id, s.topic_id, s.topic_label, s.status, s.started_at, COUNT(m.id)
             FROM sessions s LEFT JOIN messages m ON m.session_id = s.id
             GROUP BY s.id ORDER BY s.started_at DESC LIMIT 5",
        );
        assert!(recent[0].starts_with("Text(\"v016-chore\")"), "{recent:?}");
        assert!(recent[1].starts_with("Text(\"v016-free\")"), "{recent:?}");
        assert!(recent[1].ends_with("Integer(3)"), "{recent:?}");
        let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE status = 'complete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(completed >= 1);

        assert_eq!(count(&connection, "sessions"), sessions_before + 2);
        let (attempts, best): (i64, String) = connection
            .query_row(
                "SELECT attempts, best FROM chore_progress WHERE chore_id = 'market-cloth-price'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((attempts, best.as_str()), (attempts_before + 1, "350"));
        assert_sound(&connection);
    }

    fn message(speaker: Speaker, turn: u32, at: DateTime<Utc>) -> Message {
        Message {
            id: Uuid::new_v4().to_string(),
            speaker,
            content: format!("turn {turn}"),
            turn,
            created_at: at.to_rfc3339(),
        }
    }

    /// A talk started at `at`, with `answers` answers given twenty seconds
    /// apart, and completed five minutes in when `complete` is set.
    fn talk(
        database: &Database,
        topic: &str,
        at: DateTime<Utc>,
        answers: u32,
        complete: bool,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let session = Session {
            id: id.clone(),
            topic_id: topic.into(),
            topic_label: topic.into(),
            status: "active".into(),
            started_at: at.to_rfc3339(),
            completed_at: None,
            messages: Vec::new(),
        };
        database
            .create_session(&session, &message(Speaker::Ella, 0, at))
            .unwrap();
        for turn in 1..=answers {
            let when = at + Duration::seconds(20 * i64::from(turn));
            database
                .persist_turn(
                    &id,
                    &message(Speaker::Learner, turn, when),
                    &message(Speaker::Ella, turn, when),
                )
                .unwrap();
        }
        if complete {
            database
                .complete_session(&id, &(at + Duration::minutes(5)).to_rfc3339())
                .unwrap();
        }
        id
    }

    fn noon_utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    /// The laptop's own calendar day for a moment, worked out by chrono rather
    /// than SQLite, so the tests hold in whatever timezone they run in.
    fn local_day(at: DateTime<Utc>) -> String {
        at.with_timezone(&Local).format("%Y-%m-%d").to_string()
    }

    fn save_asha(database: &Database) -> Learner {
        database
            .save_learner("Asha", Some(14), "Morning Meadow", &Utc::now().to_rfc3339())
            .unwrap()
    }

    #[test]
    fn a_fresh_laptop_gets_v0_1_6_tables_plus_two_learner_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let database = Database::open(&path).unwrap();
        {
            let connection = database.connection().unwrap();
            let synchronous: i64 = connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 2, "FULL");
            for pragma in ["fullfsync", "checkpoint_fullfsync", "foreign_keys"] {
                let on: i64 = connection
                    .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
                    .unwrap();
                assert_eq!(on, 1, "{pragma}");
            }
            let journal: String = connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");

            assert!(learner_is_pinned(&connection).unwrap(), "CHECK (id = 1)");
            assert_eq!(
                rows(&connection, "SELECT name FROM pragma_table_info('learner') ORDER BY cid"),
                [
                    "id",
                    "name",
                    "age",
                    "level_name",
                    "created_at",
                    "name_spoken",
                    "interests",
                    "avatar_color",
                    "signed_out"
                ]
                .map(|column| format!("Text({column:?})"))
            );
            assert!(!has_table(&connection, "device_state").unwrap());
            for (table, column) in [
                ("sessions", "learner_id"),
                ("chore_progress", "learner_id"),
                ("character_state", "learner_id"),
                ("learner", "last_active_at"),
            ] {
                assert!(!has_column(&connection, table, column).unwrap(), "{table}.{column}");
            }

            // Table for table, what v0.1.6 made of an empty file, with only
            // the two learner columns added.
            let old = tempfile::tempdir().unwrap();
            let old_path = old.path().join("ella.sqlite3");
            let old_schema = {
                let v0_1_6 = Connection::open(&old_path).unwrap();
                create_v0_1_6_schema(&v0_1_6);
                schema(&v0_1_6)
            };
            let fresh = schema(&connection);
            assert_eq!(fresh, schema(&Database::open(&old_path).unwrap().connection().unwrap()));
            let changed = fresh
                .iter()
                .filter(|line| !old_schema.contains(line))
                .collect::<Vec<_>>();
            assert_eq!(changed.len(), 1, "{changed:?}");
            assert!(
                changed[0].contains("\"learner\"")
                    && changed[0].contains(", avatar_color TEXT, signed_out INTEGER NOT NULL DEFAULT 0"),
                "{changed:?}"
            );
        }
        assert_eq!(database.learner().unwrap(), None);
        assert_eq!(database.signed_in_learner().unwrap(), None);
        assert_eq!(database.progress().unwrap(), LearnerProgress::default());
        let first_launch = dump(&database);
        drop(database);
        assert_eq!(dump(&Database::open(&path).unwrap()), first_launch);
    }

    #[test]
    fn a_v0_1_6_laptop_keeps_every_row_and_its_learner_stays_signed_in() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let (counts, before, schema_before) = {
            let connection = Connection::open(&path).unwrap();
            create_v0_1_6_schema(&connection);
            connection.execute_batch(V0_1_6_LEARNER_DATA).unwrap();
            let counts = V0_1_6_TABLES
                .iter()
                .map(|table| count(&connection, table))
                .collect::<Vec<_>>();
            let before = V0_1_6_TABLES
                .iter()
                .filter(|table| **table != "learner")
                .flat_map(|table| rows(&connection, &format!("SELECT * FROM {table} ORDER BY rowid")))
                .collect::<Vec<_>>();
            (counts, before, schema(&connection))
        };

        let database = Database::open(&path).unwrap();
        {
            let connection = database.connection().unwrap();
            for (table, rows_before) in V0_1_6_TABLES.iter().zip(&counts) {
                assert_eq!(count(&connection, table), *rows_before, "rows lost from {table}");
            }
            // Every row outside the learner's is exactly as it was: nothing
            // was rebuilt, only two columns added.
            let after = V0_1_6_TABLES
                .iter()
                .filter(|table| **table != "learner")
                .flat_map(|table| rows(&connection, &format!("SELECT * FROM {table} ORDER BY rowid")))
                .collect::<Vec<_>>();
            assert_eq!(after, before);
            let schema_after = schema(&connection);
            let changed = schema_after
                .iter()
                .filter(|line| !schema_before.contains(line))
                .collect::<Vec<_>>();
            assert_eq!(changed.len(), 1, "only the learner table changes: {changed:?}");
            assert!(changed[0].contains("\"learner\""), "{changed:?}");
            assert_eq!(schema_after.len(), schema_before.len());
            assert!(learner_is_pinned(&connection).unwrap());
            assert!(!has_table(&connection, "device_state").unwrap());
            assert_sound(&connection);
        }

        let meera = database.signed_in_learner().unwrap().expect("still signed in");
        assert_eq!(
            meera,
            Learner {
                name: "Meera".into(),
                age: Some(15),
                level_name: "Morning Meadow".into(),
                created_at: "2026-09-01T08:00:00+00:00".into(),
                avatar_color: None,
            }
        );
        assert_eq!(database.recent_sessions(5).unwrap().len(), 3);
        let progress = database.progress().unwrap();
        assert_eq!(progress.talks_finished, 2);
        assert_eq!(progress.answers, 4);
        assert_eq!(
            progress.finished_topics,
            vec!["market-cloth-price".to_string(), "street-food".to_string()]
        );
        assert_eq!(progress.days.len(), 3);
        assert_eq!(
            database.session("s3").unwrap().messages.len(),
            3,
            "a chore transcript reads back whole"
        );
        assert_eq!(database.ledger_state("s3").unwrap(), Some((400, true)));

        // Launching again finds nothing left to do.
        let first_launch = dump(&database);
        drop(database);
        let database = Database::open(&path).unwrap();
        assert_eq!(dump(&database), first_launch);

        // Logging out survives a restart and deletes nothing.
        database.sign_out().unwrap();
        drop(database);
        let database = Database::open(&path).unwrap();
        assert_eq!(database.signed_in_learner().unwrap(), None);
        assert_eq!(database.learner().unwrap(), Some((meera.clone(), false)));
        assert_eq!(database.progress().unwrap(), progress);
        assert_eq!(database.sign_in().unwrap(), Some(meera));
        drop(database);

        // v0.1.6 can still use the file, and what it writes shows up here.
        use_as_v0_1_6(&path);
        let database = Database::open(&path).unwrap();
        let (again, signed_in) = database.learner().unwrap().unwrap();
        assert_eq!((again.name.as_str(), again.age, signed_in), ("Meera again", Some(15), true));
        assert_eq!(database.progress().unwrap().talks_finished, 3);
        assert_eq!(database.recent_sessions(5).unwrap()[0].id, "v016-chore");
    }

    #[test]
    fn a_v0_1_6_laptop_with_nobody_saved_opens_signed_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            create_v0_1_6_schema(&connection);
            // Left behind by the old "Log out", which cleared the learner but
            // not the chore counts.
            connection
                .execute(
                    "INSERT INTO chore_progress(chore_id, attempts) VALUES ('market-cloth-price', 3)",
                    [],
                )
                .unwrap();
        }
        let database = Database::open(&path).unwrap();
        assert_eq!(database.learner().unwrap(), None);
        assert_eq!(database.sign_in().unwrap(), None, "nobody to sign in");
        assert_eq!(database.learner().unwrap(), None, "and nobody made up");
        assert_eq!(count(&database.connection().unwrap(), "chore_progress"), 1);

        let asha = save_asha(&database);
        let first_launch = dump(&database);
        drop(database);
        let database = Database::open(&path).unwrap();
        assert_eq!(dump(&database), first_launch);
        assert_eq!(database.signed_in_learner().unwrap(), Some(asha));
    }

    #[test]
    fn a_first_release_laptop_opens_with_its_learner_and_zoe_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        {
            let connection = Connection::open(&path).unwrap();
            create_first_release_schema(&connection);
            connection
                .execute_batch(
                    "INSERT INTO learner(id, name, level_name, created_at)
                       VALUES (1, 'Kabir', 'Morning Meadow', '2026-05-01T08:00:00+00:00');
                     INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at)
                       VALUES ('old', 'street-food', 'Street food stories', 'complete',
                               '2026-05-02T12:00:00+00:00', '2026-05-02T12:05:00+00:00');
                     INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at)
                       VALUES ('a', 'old', 'zoe', 'Hello!', 0, '2026-05-02T12:00:00+00:00'),
                              ('b', 'old', 'learner', 'Hi Zoe', 1, '2026-05-02T12:01:00+00:00');
                     INSERT INTO skill_progress(skill_id, label, strand, updated_at)
                       VALUES ('greetings', 'Greetings', 'social', '2026-05-02T12:05:00+00:00');",
                )
                .unwrap();
        }
        let database = Database::open(&path).unwrap();
        let kabir = database.signed_in_learner().unwrap().expect("signed in");
        assert_eq!((kabir.name.as_str(), kabir.age), ("Kabir", None));
        let old = database.session("old").unwrap();
        assert_eq!(old.messages[0].speaker, Speaker::Ella);
        assert_eq!(old.messages[1].content, "Hi Zoe");
        assert_eq!(database.progress().unwrap().talks_finished, 1);
        {
            let connection = database.connection().unwrap();
            assert_eq!(count(&connection, "skill_progress"), 1);
            assert!(learner_is_pinned(&connection).unwrap());
            assert_sound(&connection);
        }

        let first_launch = dump(&database);
        drop(database);
        assert_eq!(dump(&Database::open(&path).unwrap()), first_launch);
    }

    /// The development build's two learners on file, Ravi signed in when
    /// `signed_in` names him.
    fn intermediate_laptop(path: &Path, signed_in: Option<i64>) -> (Vec<String>, Vec<String>) {
        let connection = Connection::open(path).unwrap();
        create_intermediate_schema(&connection);
        connection.execute_batch(INTERMEDIATE_DATA).unwrap();
        if let Some(id) = signed_in {
            connection
                .execute(
                    "INSERT INTO device_state(key, value) VALUES ('signed_in_learner', ?1)",
                    params![id.to_string()],
                )
                .unwrap();
        }
        assert!(!learner_is_pinned(&connection).unwrap());
        assert_sound(&connection);
        kept_rows(&connection)
    }

    /// Everything the fold back must keep exactly: every session (bar the
    /// owner it no longer needs) and every message, ledger and observation.
    fn kept_rows(connection: &Connection) -> (Vec<String>, Vec<String>) {
        let sessions = rows(
            connection,
            "SELECT id, topic_id, topic_label, status, started_at, completed_at,
                    chore_id, character_id, outcome
             FROM sessions ORDER BY id",
        );
        let mut rest = rows(connection, "SELECT * FROM messages ORDER BY id");
        rest.extend(rows(connection, "SELECT * FROM ledger_state ORDER BY session_id"));
        rest.extend(rows(connection, "SELECT * FROM observations ORDER BY id"));
        (sessions, rest)
    }

    /// The fold back of `INTERMEDIATE_DATA` came out whole: one learner, as
    /// `kept` says, and the history of both.
    fn assert_folded(database: &Database, kept: &(Vec<String>, Vec<String>), learner_row: &str) {
        let connection = database.connection().unwrap();
        assert_eq!(&kept_rows(&connection), kept, "every session and message stays");
        assert_eq!(count(&connection, "sessions"), 5);
        assert_eq!(count(&connection, "messages"), 15);

        assert!(learner_is_pinned(&connection).unwrap(), "CHECK (id = 1) is back");
        assert_eq!(rows(&connection, "SELECT * FROM learner"), vec![learner_row.to_string()]);
        assert!(!has_column(&connection, "learner", "last_active_at").unwrap());
        assert!(!has_table(&connection, "device_state").unwrap());
        let owned: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE learner_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owned, 0, "the leftover column is emptied");
        let index: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_sessions_learner'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 0);

        // Keyed by chore and character again, both learners' rows added up.
        assert!(!has_column(&connection, "chore_progress", "learner_id").unwrap());
        assert_eq!(
            rows(&connection, "SELECT * FROM chore_progress ORDER BY chore_id"),
            vec![
                "Text(\"market-cloth-price\") | Integer(3) | Text(\"2026-09-23T12:20:00+00:00\") | Text(\"400\")",
                "Text(\"metro-card-topup\") | Integer(1) | Null | Null",
            ]
        );
        assert!(!has_column(&connection, "character_state", "learner_id").unwrap());
        assert_eq!(
            rows(&connection, "SELECT * FROM character_state"),
            vec![
                "Text(\"stall-owner\") | Integer(5) | Text(\"the red scarf\") | Text(\"2026-09-24T12:20:00+00:00\") | Text(\"haggles hard\")"
            ]
        );
        assert_sound(&connection);
        drop(connection);

        // The one learner's figures are the whole laptop's history.
        assert_eq!(database.recent_sessions(5).unwrap().len(), 5);
        let progress = database.progress().unwrap();
        assert_eq!(progress.talks_finished, 3, "r3 was never answered");
        assert_eq!(progress.answers, 5);
        assert_eq!(
            progress.finished_topics,
            vec!["booking-a-cab", "market-cloth-price", "street-food"]
        );
        assert_eq!(progress.days.len(), 4);
        assert_eq!(database.ledger_state("a2").unwrap(), Some((400, true)));
    }

    const RAVI_SIGNED_IN: &str = "Integer(1) | Text(\"Ravi\") | Integer(20) | Text(\"Morning Meadow\") | Text(\"2026-09-21T08:00:00+00:00\") | Null | Text(\"films\") | Text(\"#FF8800\") | Integer(0)";
    const ASHA_SIGNED_OUT: &str = "Integer(1) | Text(\"Asha\") | Integer(14) | Text(\"Morning Meadow\") | Text(\"2026-09-20T08:00:00+00:00\") | Text(\"AH-sha\") | Text(\"cricket\") | Text(\"#7C5CFF\") | Integer(1)";

    #[test]
    fn the_multi_learner_layout_folds_back_into_whoever_was_signed_in() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        // Ravi is signed in, though Asha was active more recently and has
        // the lower id: the pointer decides.
        let kept = intermediate_laptop(&path, Some(2));

        let database = Database::open(&path).unwrap();
        assert_folded(&database, &kept, RAVI_SIGNED_IN);
        let ravi = database.signed_in_learner().unwrap().expect("still signed in");
        assert_eq!(
            ravi,
            Learner {
                name: "Ravi".into(),
                age: Some(20),
                level_name: "Morning Meadow".into(),
                created_at: "2026-09-21T08:00:00+00:00".into(),
                avatar_color: Some("#FF8800".into()),
            }
        );

        // Done once: the next launch finds nothing to fold.
        let first_launch = dump(&database);
        drop(database);
        let database = Database::open(&path).unwrap();
        assert_eq!(dump(&database), first_launch);
        drop(database);

        // And v0.1.6 can use the file it left.
        use_as_v0_1_6(&path);
        let database = Database::open(&path).unwrap();
        assert_eq!(database.signed_in_learner().unwrap().unwrap().name, "Ravi again");
        assert_eq!(database.progress().unwrap().talks_finished, 4);
    }

    #[test]
    fn the_multi_learner_layout_with_nobody_signed_in_folds_back_signed_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let kept = intermediate_laptop(&path, None);

        let database = Database::open(&path).unwrap();
        // Nobody was signed in, so the most recently active learner stays,
        // and stays logged out.
        assert_folded(&database, &kept, ASHA_SIGNED_OUT);
        assert_eq!(database.signed_in_learner().unwrap(), None);
        let (asha, signed_in) = database.learner().unwrap().unwrap();
        assert_eq!((asha.name.as_str(), signed_in), ("Asha", false));

        let first_launch = dump(&database);
        drop(database);
        let database = Database::open(&path).unwrap();
        assert_eq!(dump(&database), first_launch);

        // Logging in brings her back to the whole history.
        assert_eq!(database.sign_in().unwrap(), Some(asha));
        assert_eq!(database.progress().unwrap().talks_finished, 3);
    }

    #[test]
    fn a_pointer_to_a_learner_who_is_not_there_counts_as_signed_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let kept = intermediate_laptop(&path, Some(7));
        let database = Database::open(&path).unwrap();
        assert_folded(&database, &kept, ASHA_SIGNED_OUT);
    }

    #[test]
    fn the_developer_laptop_in_the_multi_learner_layout_comes_back_as_it_was() {
        // How the one real database in that layout got there: v0.1.6 wrote
        // it, with one learner and one talk ended before they answered, and
        // the development build then moved it into its own layout.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let kept = {
            let connection = Connection::open(&path).unwrap();
            create_v0_1_6_schema(&connection);
            connection
                .execute_batch(
                    "INSERT INTO learner(id, name, age, level_name, created_at)
                       VALUES (1, 'Souvik', 27, 'Morning Meadow', '2026-09-25T09:22:30.294180+00:00');
                     INSERT INTO sessions(id, topic_id, topic_label, status, started_at, completed_at)
                       VALUES ('only', 'street-food', 'Street food stories', 'complete',
                               '2026-09-25T09:22:54.461353+00:00', '2026-09-25T09:23:30+00:00');
                     INSERT INTO messages(id, session_id, speaker, content, turn_number, created_at)
                       VALUES ('hello', 'only', 'ella', 'What did you eat today?', 0,
                               '2026-09-25T09:22:54.461353+00:00');",
                )
                .unwrap();
            upgrade_v0_1_6_to_intermediate(&connection);
            assert!(has_table(&connection, "device_state").unwrap());
            assert!(has_column(&connection, "chore_progress", "learner_id").unwrap());
            assert_eq!(count(&connection, "device_state"), 1);
            kept_rows(&connection)
        };

        let database = Database::open(&path).unwrap();
        {
            let connection = database.connection().unwrap();
            assert_eq!(&kept_rows(&connection), &kept);
            assert!(learner_is_pinned(&connection).unwrap());
            assert!(!has_table(&connection, "device_state").unwrap());
            assert!(!has_column(&connection, "chore_progress", "learner_id").unwrap());
            assert_sound(&connection);
        }
        let souvik = database.signed_in_learner().unwrap().expect("still signed in");
        assert_eq!((souvik.name.as_str(), souvik.age), ("Souvik", Some(27)));
        assert_eq!(souvik.created_at, "2026-09-25T09:22:30.294180+00:00");
        assert_eq!(database.recent_sessions(5).unwrap().len(), 1);
        assert_eq!(database.session("only").unwrap().messages.len(), 1);
        assert_eq!(
            database.progress().unwrap(),
            LearnerProgress::default(),
            "a talk with no answers counts for nothing"
        );
        let first_launch = dump(&database);
        drop(database);
        assert_eq!(dump(&Database::open(&path).unwrap()), first_launch);
    }

    #[test]
    fn a_crash_part_way_through_the_fold_back_leaves_it_to_the_next_launch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let kept = intermediate_laptop(&path, Some(2));

        // Run every step of the fold back and stop short of the commit, then
        // copy the files as they stand: what the app being killed at that
        // moment would leave on disk.
        let crashed = tempfile::tempdir().unwrap();
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch("PRAGMA foreign_keys = OFF; BEGIN IMMEDIATE;")
                .unwrap();
            fold_into_one_learner(&connection).unwrap();
            assert!(!has_table(&connection, "device_state").unwrap(), "the work was done");
            for file in ["ella.sqlite3", "ella.sqlite3-wal"] {
                let from = directory.path().join(file);
                if from.exists() {
                    fs::copy(from, crashed.path().join(file)).unwrap();
                }
            }
        }

        // The copy still has the old layout whole, and opening it finishes
        // the job.
        let copy = crashed.path().join("ella.sqlite3");
        {
            let connection = Connection::open(&copy).unwrap();
            assert!(has_table(&connection, "device_state").unwrap());
            assert!(!learner_is_pinned(&connection).unwrap());
            assert_eq!(count(&connection, "learner"), 2);
        }
        let recovered = Database::open(&copy).unwrap();
        assert_folded(&recovered, &kept, RAVI_SIGNED_IN);

        // So does the original, whose unfinished fold was rolled back.
        let database = Database::open(&path).unwrap();
        assert_folded(&database, &kept, RAVI_SIGNED_IN);
        assert_eq!(dump(&database), dump(&recovered));
    }

    #[test]
    fn progress_counts_the_whole_history_not_the_five_newest() {
        let database = Database::in_memory().unwrap();
        save_asha(&database);
        // A week of one two-answer talk a day.
        let week = (0..7).map(|offset| noon_utc(2026, 1, 10 + offset)).collect::<Vec<_>>();
        let topics = [
            "street-food",
            "restaurant-order",
            "booking-a-cab",
            "street-food",
            "doctor-clinic",
            "asking-directions",
            "market-bargaining",
        ];
        for (at, topic) in week.iter().zip(topics) {
            talk(&database, topic, *at, 2, true);
        }
        let progress = database.progress().unwrap();
        assert_eq!(progress.days.len(), 7);
        let expected_days = week.iter().rev().map(|at| local_day(*at)).collect::<Vec<_>>();
        assert_eq!(
            progress.days.iter().map(|day| day.day.clone()).collect::<Vec<_>>(),
            expected_days,
            "every talk day, newest first"
        );
        assert!(progress.days.iter().all(|day| day.talks == 1 && day.answers == 2));
        assert_eq!(progress.talks_finished, 7);
        assert_eq!(progress.answers, 14);
        assert_eq!(
            progress.finished_topics,
            vec![
                "asking-directions",
                "booking-a-cab",
                "doctor-clinic",
                "market-bargaining",
                "restaurant-order",
                "street-food"
            ]
        );
        // The home list still stops at five; the figures above did not.
        assert_eq!(database.recent_sessions(5).unwrap().len(), 5);

        // Three more talks on the last day add to it and take nothing away.
        let last = week[6];
        for minute in 1..=3 {
            talk(&database, "job-interview", last + Duration::seconds(30 * minute), 1, true);
        }
        let busier = database.progress().unwrap();
        assert_eq!(busier.days.len(), 7);
        assert_eq!(busier.days[0].day, local_day(last));
        assert_eq!((busier.days[0].talks, busier.days[0].answers), (4, 5));
        assert_eq!(busier.days[1..], progress.days[1..]);
        assert_eq!(busier.talks_finished, 10);
        assert_eq!(busier.answers, 17);
        assert!(busier.finished_topics.contains(&"job-interview".to_string()));

        // A talk closed before anything was said changes nothing at all.
        talk(&database, "doctor-clinic", noon_utc(2026, 1, 20), 0, true);
        assert_eq!(database.progress().unwrap(), busier);

        // An unfinished talk with an answer is a talk day and an answer, but
        // not a finished talk.
        talk(&database, "doctor-clinic", noon_utc(2026, 1, 21), 1, false);
        let open = database.progress().unwrap();
        assert_eq!(open.days.len(), 8);
        assert_eq!(open.days[0].day, local_day(noon_utc(2026, 1, 21)));
        assert_eq!((open.days[0].talks, open.days[0].answers), (1, 1));
        assert_eq!(open.talks_finished, 10);
        assert_eq!(open.answers, 18);

        // Logging out and back in changes none of it.
        database.sign_out().unwrap();
        database.sign_in().unwrap();
        assert_eq!(database.progress().unwrap(), open);
    }

    #[test]
    fn a_talk_past_midnight_counts_once_on_the_day_it_began() {
        let database = Database::in_memory().unwrap();
        save_asha(&database);
        let local = |day: u32, hour: u32, minute: u32| {
            Local
                .from_local_datetime(
                    &NaiveDate::from_ymd_opt(2026, 1, day)
                        .unwrap()
                        .and_hms_opt(hour, minute, 0)
                        .unwrap(),
                )
                .earliest()
                .unwrap()
                .with_timezone(&Utc)
        };
        let evening = local(15, 23, 50);
        let after_midnight = local(16, 0, 10);
        let id = talk(&database, "street-food", evening, 1, false);
        database
            .persist_turn(
                &id,
                &message(Speaker::Learner, 2, after_midnight),
                &message(Speaker::Ella, 2, after_midnight),
            )
            .unwrap();
        database
            .complete_session(&id, &after_midnight.to_rfc3339())
            .unwrap();

        let progress = database.progress().unwrap();
        assert_eq!(
            progress.days,
            vec![
                DayActivity { day: "2026-01-16".into(), talks: 0, answers: 1 },
                DayActivity { day: "2026-01-15".into(), talks: 1, answers: 1 },
            ]
        );
        assert_eq!(progress.days.iter().map(|day| day.talks).sum::<u32>(), 1);
        assert_eq!(progress.talks_finished, 1);
    }

    #[test]
    fn log_out_changes_one_flag_and_log_in_changes_it_back() {
        let database = Database::in_memory().unwrap();
        save_asha(&database);
        let session = talk(&database, "street-food", noon_utc(2026, 1, 10), 2, true);
        let asha = database.save_avatar_color("#FF8800").unwrap().unwrap();
        assert_eq!(asha.avatar_color.as_deref(), Some("#FF8800"));
        let before = dump(&database);

        database.sign_out().unwrap();
        assert_eq!(database.signed_in_learner().unwrap(), None);
        assert_eq!(database.learner().unwrap(), Some((asha.clone(), false)));
        let after = dump(&database);
        let changed = before
            .iter()
            .filter(|line| !after.contains(line))
            .collect::<Vec<_>>();
        assert_eq!(changed.len(), 1, "only the learner row changes: {changed:?}");
        assert!(changed[0].starts_with("learner:") && changed[0].ends_with("Integer(0)"));
        assert_eq!(after.len(), before.len(), "nothing was deleted");
        assert_eq!(database.session(&session).unwrap().messages.len(), 5);

        // Signed out, the colour cannot be changed.
        assert_eq!(database.save_avatar_color("#000000").unwrap(), None);
        assert_eq!(dump(&database), after);

        assert_eq!(database.sign_in().unwrap(), Some(asha.clone()));
        assert_eq!(dump(&database), before);
        assert_eq!(database.progress().unwrap().talks_finished, 1);
    }

    #[test]
    fn the_name_step_keeps_when_the_learner_started_and_their_colour() {
        let database = Database::in_memory().unwrap();
        let asha = save_asha(&database);
        database.save_avatar_color("#FF8800").unwrap();
        database.sign_out().unwrap();

        let again = database
            .save_learner("Asha Rao", None, "Morning Meadow", "2030-01-01T00:00:00+00:00")
            .unwrap();
        assert_eq!(
            again,
            Learner {
                name: "Asha Rao".into(),
                age: Some(14),
                avatar_color: Some("#FF8800".into()),
                ..asha
            }
        );
        assert_eq!(database.signed_in_learner().unwrap(), Some(again));
    }

    #[test]
    fn a_saved_talk_survives_the_app_being_killed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let wal = directory.path().join("ella.sqlite3-wal");
        let database = Database::open(&path).unwrap();
        let asha = save_asha(&database);
        let session = talk(&database, "street-food", noon_utc(2026, 1, 10), 2, true);
        assert!(
            fs::metadata(&wal).unwrap().len() > 0,
            "the writes are still in the log, which is the case worth testing"
        );

        // Copy the files as they stand with the app still running and the
        // connection never closed: what a crash or a force-quit leaves on
        // disk. A power cut is a matter for the drive's cache, which is what
        // `fullfsync` is for; a test cannot pull the plug.
        let crashed = tempfile::tempdir().unwrap();
        fs::copy(&path, crashed.path().join("ella.sqlite3")).unwrap();
        fs::copy(&wal, crashed.path().join("ella.sqlite3-wal")).unwrap();
        let recovered = Database::open(&crashed.path().join("ella.sqlite3")).unwrap();
        assert_eq!(recovered.signed_in_learner().unwrap(), Some(asha.clone()));
        assert_eq!(recovered.session(&session).unwrap().messages.len(), 5);
        assert_eq!(recovered.progress().unwrap().talks_finished, 1);
        drop(recovered);
        drop(database);

        // And a plain restart of the real file sees the same.
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.signed_in_learner().unwrap(), Some(asha));
        assert_eq!(reopened.session(&session).unwrap().messages.len(), 5);
    }

    #[test]
    fn a_checkpoint_leaves_the_whole_history_in_the_main_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ella.sqlite3");
        let wal = directory.path().join("ella.sqlite3-wal");
        let database = Database::open(&path).unwrap();
        save_asha(&database);
        talk(&database, "street-food", noon_utc(2026, 1, 10), 2, true);

        database.checkpoint().unwrap();
        let log = fs::metadata(&wal).map(|metadata| metadata.len()).unwrap_or(0);
        assert_eq!(log, 0, "the log is emptied");
        database.checkpoint().unwrap();

        // Only the main file, with no log beside it to lean on.
        let copied = tempfile::tempdir().unwrap();
        let alone = copied.path().join("ella.sqlite3");
        fs::copy(&path, &alone).unwrap();
        let connection = Connection::open(&alone).unwrap();
        let name: String = connection
            .query_row("SELECT name FROM learner", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "Asha");
        assert_eq!(count(&connection, "messages"), 5);

        // Writing after a checkpoint still works and is still durable.
        talk(&database, "booking-a-cab", noon_utc(2026, 1, 11), 1, true);
        drop(database);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.progress().unwrap().talks_finished, 2);
    }
}
