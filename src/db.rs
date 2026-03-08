use rusqlite::{Connection, Result, params};
use std::path::Path;

/// SQLite database for transcription history and settings.
pub struct Db {
    conn: Connection,
}

/// A single transcription record.
pub struct Transcription {
    pub _id: i64,
    pub text: String,
    pub created_at: String,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn insert(&self, text: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO transcriptions (text) VALUES (?1)",
            params![text],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query_map(params![key], |row| row.get(0))?;
        match rows.next() {
            Some(Ok(val)) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Transcription>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, created_at FROM transcriptions ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(Transcription {
                _id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        rows.collect()
    }
}
