use rusqlite::{Connection, Result, params};
use std::path::Path;

/// Allowed setting keys (prevents arbitrary key/value injection).
/// Allowed setting key prefixes (prevents arbitrary key/value injection).
const VALID_SETTING_PREFIXES: &[&str] = &[
    "transcription_mode",
    "window_x",
    "window_y",
    "api_custom_url",
    "api_custom_key",
    "api_custom_model",
    "api_key_",
    "tts_provider",
    "tts_voice",
    "tts_api_preset",
    "tts_api_url",
    "tts_api_key",
    "tts_api_model",
];

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
        // Validate key against whitelist
        if !VALID_SETTING_PREFIXES
            .iter()
            .any(|p| key == *p || key.starts_with(p))
        {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "unknown setting key: {key}"
            )));
        }
        // Cap value length to prevent abuse
        let value = &value[..value.len().min(4096)];
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
