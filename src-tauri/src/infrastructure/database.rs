use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{
    domain::{
        skill_seeds, stage_label, Garden, Learner, Message, Session, SessionListItem, SkillProgress,
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
             CREATE TABLE IF NOT EXISTS skill_progress (
               skill_id TEXT PRIMARY KEY,
               label TEXT NOT NULL,
               strand TEXT NOT NULL,
               evidence_count INTEGER NOT NULL DEFAULT 0,
               last_evidence TEXT,
               updated_at TEXT NOT NULL
             );",
        )?;
        rename_legacy_speaker(&connection)?;
        add_learner_age(&connection)?;
        seed_skills(&connection)?;
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
        skill_id: &str,
        evidence: &str,
        now: &str,
    ) -> EllaResult<SkillProgress> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        insert_message(&transaction, session_id, learner)?;
        insert_message(&transaction, session_id, ella)?;
        transaction.execute(
            "UPDATE skill_progress
             SET evidence_count = evidence_count + 1, last_evidence = ?2, updated_at = ?3
             WHERE skill_id = ?1",
            params![skill_id, evidence, now],
        )?;
        let skill = read_skill(&transaction, skill_id)?;
        transaction.commit()?;
        Ok(skill)
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

    pub fn garden(&self) -> EllaResult<Garden> {
        let connection = self.connection()?;
        let total_conversations = connection.query_row(
            "SELECT COUNT(*) FROM sessions WHERE status = 'complete'",
            [],
            |row| row.get::<_, u32>(0),
        )?;
        let mut statement = connection.prepare(
            "SELECT skill_id, label, strand, evidence_count, last_evidence
             FROM skill_progress
             ORDER BY CASE strand WHEN 'vocabulary' THEN 1 WHEN 'grammar' THEN 2 ELSE 3 END",
        )?;
        let skills = statement
            .query_map([], skill_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Garden {
            level_name: "Morning Meadow".into(),
            total_conversations,
            skills,
        })
    }

    pub fn reset(&self) -> EllaResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM messages", [])?;
        transaction.execute("DELETE FROM sessions", [])?;
        transaction.execute("DELETE FROM learner", [])?;
        transaction.execute("DELETE FROM skill_progress", [])?;
        seed_skills(&transaction)?;
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

fn seed_skills(connection: &Connection) -> EllaResult<()> {
    for (id, label, strand) in skill_seeds() {
        connection.execute(
            "INSERT OR IGNORE INTO skill_progress(skill_id, label, strand, evidence_count, updated_at)
             VALUES(?1, ?2, ?3, 0, datetime('now'))",
            params![id, label, strand],
        )?;
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

fn read_skill(connection: &Connection, id: &str) -> EllaResult<SkillProgress> {
    connection
        .query_row(
            "SELECT skill_id, label, strand, evidence_count, last_evidence
             FROM skill_progress WHERE skill_id = ?1",
            [id],
            skill_from_row,
        )
        .map_err(Into::into)
}

fn skill_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillProgress> {
    let evidence_count = row.get::<_, u32>(3)?;
    let stage = evidence_count.min(3) as u8;
    Ok(SkillProgress {
        id: row.get(0)?,
        label: row.get(1)?,
        strand: row.get(2)?,
        evidence_count,
        stage,
        stage_label: stage_label(stage).into(),
        last_evidence: row.get(4)?,
    })
}
