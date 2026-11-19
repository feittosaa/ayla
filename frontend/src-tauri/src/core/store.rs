use rusqlite::{params, Connection};
use serde::Serialize;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub struct Store {
    conn: Connection,
}

#[derive(Clone, Debug, Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub cloud_consent: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub complete: bool,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if !path.exists() {
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(path)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if version == 0 {
            let tx = conn.transaction()?;
            tx.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL, cloud_consent INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE messages(id INTEGER PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT NOT NULL, complete INTEGER NOT NULL);
                CREATE INDEX messages_conversation ON messages(conversation_id,id);
                CREATE TABLE models(id TEXT PRIMARY KEY, definition TEXT NOT NULL);
                CREATE TABLE metrics(id INTEGER PRIMARY KEY, model_id TEXT NOT NULL, first_token_ms INTEGER, duration_ms INTEGER NOT NULL, tokens INTEGER NOT NULL, cancelled INTEGER NOT NULL);
                CREATE TABLE memories(id TEXT PRIMARY KEY, content TEXT NOT NULL, model_id TEXT NOT NULL, vector TEXT NOT NULL);
                PRAGMA user_version=1;")?;
            tx.commit()?;
        }
        Ok(Self { conn })
    }
    pub fn create_conversation(&self, title: &str) -> rusqlite::Result<Conversation> {
        let conversation = Conversation {
            id: id(),
            title: title.chars().take(120).collect(),
            cloud_consent: false,
        };
        self.conn.execute(
            "INSERT INTO conversations(id,title) VALUES(?1,?2)",
            params![conversation.id, conversation.title],
        )?;
        Ok(conversation)
    }
    pub fn conversations(&self) -> rusqlite::Result<Vec<Conversation>> {
        self.conn
            .prepare("SELECT id,title,cloud_consent FROM conversations ORDER BY rowid DESC")?
            .query_map([], |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    cloud_consent: r.get(2)?,
                })
            })?
            .collect()
    }
    pub fn consent(&self, id: &str) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT cloud_consent FROM conversations WHERE id=?1",
            [id],
            |r| r.get(0),
        )
    }
    pub fn set_consent(&self, id: &str, consent: bool) -> rusqlite::Result<()> {
        if self.conn.execute(
            "UPDATE conversations SET cloud_consent=?2 WHERE id=?1",
            params![id, consent],
        )? != 1
        {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }
    pub fn messages(&self, id: &str) -> rusqlite::Result<Vec<Message>> {
        let mut rows:Vec<Message> = self.conn.prepare("SELECT id,role,content,complete FROM messages WHERE conversation_id=?1 ORDER BY id DESC LIMIT 200")?
            .query_map([id], |r| Ok(Message { id:r.get(0)?, role:r.get(1)?, content:r.get(2)?, complete:r.get(3)? }))?.collect::<rusqlite::Result<_>>()?;
        rows.reverse();
        Ok(rows)
    }
    pub fn context(
        &self,
        id: &str,
        limit: usize,
        chars: usize,
    ) -> rusqlite::Result<Vec<serde_json::Value>> {
        // Limitar no SQL antes de alocar; nunca misturar conversas ou respostas parciais.
        let mut rows: Vec<(String,String)> = self.conn.prepare("SELECT role,content FROM messages WHERE conversation_id=?1 AND complete=1 ORDER BY id DESC LIMIT ?2")?
            .query_map(params![id, limit as i64], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut remaining = chars;
        rows.retain(|(_, content)| {
            let count = content.chars().count();
            if count > remaining {
                remaining = 0;
                false
            } else {
                remaining -= count;
                true
            }
        });
        rows.reverse();
        Ok(rows
            .into_iter()
            .map(|(role, content)| serde_json::json!({"role":role,"content":content}))
            .collect())
    }
    pub fn save_turn(
        &self,
        id: &str,
        user: &str,
        assistant: &str,
        complete: bool,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO messages(conversation_id,role,content,complete) VALUES(?1,'user',?2,1)",
            params![id, user],
        )?;
        tx.execute("INSERT INTO messages(conversation_id,role,content,complete) VALUES(?1,'assistant',?2,?3)",params![id,assistant,complete])?;
        tx.commit()
    }
    #[cfg(test)]
    pub fn add_message(
        &self,
        id: &str,
        role: &str,
        content: &str,
        complete: bool,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO messages(conversation_id,role,content,complete) VALUES(?1,?2,?3,?4)",
            params![id, role, content, complete],
        )?;
        Ok(())
    }
    pub fn save_model(&self, id: &str, definition: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO models VALUES(?1,?2)",
            params![id, definition],
        )?;
        Ok(())
    }
    pub fn models(&self) -> rusqlite::Result<Vec<String>> {
        self.conn
            .prepare("SELECT definition FROM models ORDER BY rowid")?
            .query_map([], |r| r.get(0))?
            .collect()
    }
    pub fn remove_model(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM models WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn metric(
        &self,
        model: &str,
        first: Option<u64>,
        duration: u64,
        tokens: u64,
        cancelled: bool,
    ) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO metrics(model_id,first_token_ms,duration_ms,tokens,cancelled) VALUES(?1,?2,?3,?4,?5)",params![model,first.map(|v|v as i64),duration as i64,tokens as i64,cancelled])?;
        self.conn.execute("DELETE FROM metrics WHERE id NOT IN (SELECT id FROM metrics ORDER BY id DESC LIMIT 1000)",[])?;
        Ok(())
    }
    pub fn metrics(&self) -> rusqlite::Result<Vec<serde_json::Value>> {
        self.conn.prepare("SELECT model_id,first_token_ms,duration_ms,tokens,cancelled FROM metrics ORDER BY id DESC LIMIT 50")?.query_map([],|r| Ok(serde_json::json!({"model":r.get::<_,String>(0)?,"first_token_ms":r.get::<_,Option<i64>>(1)?,"duration_ms":r.get::<_,i64>(2)?,"chunks":r.get::<_,i64>(3)?,"cancelled":r.get::<_,bool>(4)?})))?.collect()
    }
    pub fn save_memory(&self, content: &str, model: &str, vector: &[f32]) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO memories VALUES(?1,?2,?3,?4)",
            params![id(), content, model, serde_json::to_string(vector).unwrap()],
        )?;
        Ok(())
    }
    pub fn memories(&self, model: &str) -> rusqlite::Result<Vec<(String, Vec<f32>)>> {
        self.conn.prepare("SELECT content,vector FROM memories WHERE model_id=?1 ORDER BY rowid DESC LIMIT 5000")?.query_map([model],|r|{
            let content:String=r.get(0)?; let raw:String=r.get(1)?;
            Ok((content,serde_json::from_str(&raw).unwrap_or_default()))
        })?.collect()
    }
    pub fn import_legacy(&self, source: &Path) -> rusqlite::Result<usize> {
        let legacy =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut statement = legacy.prepare("SELECT role,content FROM messages ORDER BY id")?;
        let rows =
            statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let tx = self.conn.unchecked_transaction()?;
        let conversation = id();
        tx.execute(
            "INSERT INTO conversations(id,title) VALUES(?1,'Histórico importado')",
            [&conversation],
        )?;
        let mut count = 0;
        for row in rows {
            let (role, content) = row?;
            if !matches!(role.as_str(), "user" | "assistant") || content.trim().is_empty() {
                continue;
            }
            tx.execute(
                "INSERT INTO messages(conversation_id,role,content,complete) VALUES(?1,?2,?3,1)",
                params![conversation, role, content],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }
    pub fn backup(&self, destination: &Path) -> rusqlite::Result<()> {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(destination)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn
            .backup("main", destination, rusqlite::backup::NO_PROGRESS)
    }
    #[cfg(test)]
    pub fn version(&self) -> rusqlite::Result<u32> {
        self.conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_preserves_source_and_backup_restores_native_data() {
        let root = std::env::temp_dir().join(format!("ayla-store-contract-{}", id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("legacy.db");
        let legacy = Connection::open(&source).unwrap();
        legacy.execute_batch("CREATE TABLE messages(id INTEGER PRIMARY KEY,role TEXT,content TEXT); INSERT INTO messages VALUES(1,'user','legacy fixture'),(2,'assistant',''),(3,'assistant','answer');").unwrap();
        drop(legacy);
        let original = std::fs::read(&source).unwrap();
        let db = Store::open(&root.join("native.db")).unwrap();
        assert_eq!(db.import_legacy(&source).unwrap(), 2);
        assert_eq!(original, std::fs::read(&source).unwrap());
        let backup = root.join("backup.db");
        db.backup(&backup).unwrap();
        let restored = Store::open(&backup).unwrap();
        let count: i64 = restored
            .conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        assert!(db.backup(&backup).is_err());
        drop(restored);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn migrations_are_idempotent_and_conversations_are_isolated() {
        let path = std::env::temp_dir().join(format!("ayla-store-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let a = db.create_conversation("A").unwrap();
        let b = db.create_conversation("B").unwrap();
        db.add_message(&a.id, "user", "apenas A", true).unwrap();
        db.add_message(&a.id, "assistant", "parcial", false)
            .unwrap();
        db.add_message(&b.id, "user", "apenas B", true).unwrap();
        assert_eq!(db.context(&a.id, 10, 100).unwrap().len(), 1);
        assert!(!serde_json::to_string(&db.context(&a.id, 10, 100).unwrap())
            .unwrap()
            .contains("B"));
        assert!(db.context(&a.id, 10, 1).unwrap().is_empty());
        assert!(!db.consent(&a.id).unwrap());
        drop(db);
        assert_eq!(Store::open(&path).unwrap().version().unwrap(), 1);
        std::fs::remove_file(path).unwrap();
    }
}
