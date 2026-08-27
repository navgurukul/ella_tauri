use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{
    domain::{
        Learner, Message, Session, SessionListItem,
    },
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
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
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
",
        )?;
        rename_legacy_speaker(&connection)?;
        add_learner_age(&connection)?;
        widen_message_speaker(&connection)?;
        add_chore_tables(&connection)?;
        Ok(())
    }

    pub fn learner(&self) -> EllaResult<Option<Learner>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT name, age, level_name, created_at FROM learner WHERE id = 1",
                [],
                |row| {
                    Ok(Learner {
                        name: row.get(0)?,
                        age: row.get(1)?,
                        level_name: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_learner(&self, learner: &Learner) -> EllaResult<()> {
        self.connection()?.execute(
            "INSERT INTO learner(id, name, age, level_name, created_at)
             VALUES(1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               age = COALESCE(excluded.age, learner.age),
               level_name = excluded.level_name",
            params![
                learner.name,
                learner.age,
                learner.level_name,
                learner.created_at
            ],
        )?;
        Ok(())
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

    /// How many conversations the learner has finished. The garden that used to
    /// render progress is gone; this count survives because the home screen and
    /// the session summary both still show it.
    pub fn completed_conversations(&self) -> EllaResult<u32> {
        Ok(self.connection()?.query_row(
            "SELECT COUNT(*) FROM sessions WHERE status = 'complete'",
            [],
            |row| row.get::<_, u32>(0),
        )?)
    }

    pub fn reset(&self) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM messages", [])?;
        transaction.execute("DELETE FROM sessions", [])?;
        transaction.execute("DELETE FROM learner", [])?;
        transaction.commit()?;
        Ok(())
    }
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
