use super::memory::{self, Fact, Privacy, Turn};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

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
        if version > 2 {
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
        if version == 1 {
            // Backup consistente antes de alterar um banco com dados do usuário.
            let backups = path.parent().unwrap_or(Path::new(".")).join("backups");
            std::fs::create_dir_all(&backups)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Self { conn }.backup(&backups.join(format!("ayla-antes-v2-{}.db", id())))?;
            conn = Connection::open(path)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
        }
        if version <= 1 {
            Self::migrate_v2(&mut conn)?;
        }
        Ok(Self { conn })
    }
    /// v2: memória longa (fatos, vetores binários por modelo, índice FTS5) e
    /// cursor de extração por conversa. Notas antigas viram fatos manuais.
    fn migrate_v2(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE facts(id TEXT PRIMARY KEY, content TEXT NOT NULL, kind TEXT NOT NULL, privacy TEXT NOT NULL, origin TEXT NOT NULL, conversation_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX facts_conversation ON facts(conversation_id);
            CREATE TABLE fact_vectors(fact_id TEXT NOT NULL REFERENCES facts(id) ON DELETE CASCADE, model_id TEXT NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(fact_id, model_id));
            CREATE VIRTUAL TABLE facts_fts USING fts5(content, content='facts', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2');
            CREATE TRIGGER facts_ai AFTER INSERT ON facts BEGIN INSERT INTO facts_fts(rowid,content) VALUES(new.rowid,new.content); END;
            CREATE TRIGGER facts_ad AFTER DELETE ON facts BEGIN INSERT INTO facts_fts(facts_fts,rowid,content) VALUES('delete',old.rowid,old.content); END;
            CREATE TRIGGER facts_au AFTER UPDATE OF content ON facts BEGIN INSERT INTO facts_fts(facts_fts,rowid,content) VALUES('delete',old.rowid,old.content); INSERT INTO facts_fts(rowid,content) VALUES(new.rowid,new.content); END;
            ALTER TABLE conversations ADD COLUMN memory_cursor INTEGER NOT NULL DEFAULT 0;")?;
        let now = now();
        // Vetores antigos foram gerados sem os prefixos de busca; o worker de memória
        // recalcula todos com o modelo atual (backfill), mantendo o texto original.
        tx.execute(
            "INSERT INTO facts SELECT id,content,'nota','uncertain','manual',NULL,?1,?1 FROM memories ORDER BY rowid",
            [now],
        )?;
        // Conversas anteriores não são reprocessadas: memória automática vale daqui em diante.
        tx.execute_batch("UPDATE conversations SET memory_cursor=COALESCE((SELECT MAX(id) FROM messages WHERE conversation_id=conversations.id),0); PRAGMA user_version=2;")?;
        tx.commit()
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
    pub fn insert_fact(
        &self,
        content: &str,
        kind: &str,
        privacy: Privacy,
        origin: &str,
        conversation: Option<&str>,
    ) -> rusqlite::Result<Fact> {
        let now = now();
        let fact = Fact {
            id: id(),
            content: content.chars().take(1000).collect(),
            kind: kind.into(),
            privacy,
            origin: origin.into(),
            conversation_id: conversation.map(Into::into),
            created_at: now,
            updated_at: now,
        };
        self.conn.execute(
            "INSERT INTO facts VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
            params![fact.id, fact.content, fact.kind, privacy.as_str(), fact.origin, fact.conversation_id, now],
        )?;
        Ok(fact)
    }
    /// Altera texto e/ou privacidade; mudar o texto descarta os vetores antigos.
    pub fn update_fact(&self, id: &str, content: Option<&str>, privacy: Option<Privacy>) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut changed = 0;
        if let Some(content) = content {
            let content: String = content.chars().take(1000).collect();
            changed += tx.execute("UPDATE facts SET content=?2, updated_at=?3 WHERE id=?1 AND content<>?2", params![id, content, now()])?;
            if changed > 0 {
                tx.execute("DELETE FROM fact_vectors WHERE fact_id=?1", [id])?;
            }
        }
        if let Some(privacy) = privacy {
            changed += tx.execute("UPDATE facts SET privacy=?2, updated_at=?3 WHERE id=?1", params![id, privacy.as_str(), now()])?;
        }
        if changed == 0 && !tx.query_row("SELECT EXISTS(SELECT 1 FROM facts WHERE id=?1)", [id], |r| r.get::<_, bool>(0))? {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.commit()
    }
    pub fn delete_fact(&self, id: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM facts WHERE id=?1", [id])? == 1)
    }
    fn fact_row(r: &rusqlite::Row) -> rusqlite::Result<Fact> {
        Ok(Fact {
            id: r.get(0)?,
            content: r.get(1)?,
            kind: r.get(2)?,
            privacy: Privacy::parse(&r.get::<_, String>(3)?),
            origin: r.get(4)?,
            conversation_id: r.get(5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        })
    }
    pub fn facts(&self) -> rusqlite::Result<Vec<Fact>> {
        self.conn
            .prepare("SELECT id,content,kind,privacy,origin,conversation_id,created_at,updated_at FROM facts ORDER BY updated_at DESC, rowid DESC LIMIT 5000")?
            .query_map([], Self::fact_row)?
            .collect()
    }
    pub fn facts_by_ids(&self, ids: &[String]) -> rusqlite::Result<Vec<Fact>> {
        let mut out = Vec::new();
        let mut statement = self.conn.prepare("SELECT id,content,kind,privacy,origin,conversation_id,created_at,updated_at FROM facts WHERE id=?1")?;
        for id in ids {
            if let Some(fact) = statement.query_map([id], Self::fact_row)?.next() {
                out.push(fact?);
            }
        }
        Ok(out)
    }
    /// Memórias criadas automaticamente ou a pedido nesta conversa (para a nota com desfazer).
    pub fn conversation_facts(&self, conversation: &str) -> rusqlite::Result<Vec<Fact>> {
        self.conn
            .prepare("SELECT id,content,kind,privacy,origin,conversation_id,created_at,updated_at FROM facts WHERE conversation_id=?1 AND origin<>'manual' ORDER BY rowid")?
            .query_map([conversation], Self::fact_row)?
            .collect()
    }
    pub fn set_vector(&self, fact: &str, model: &str, vector: &[f32]) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO fact_vectors VALUES(?1,?2,?3)",
            params![fact, model, memory::encode(vector)],
        )?;
        Ok(())
    }
    pub fn vectors(&self, model: &str) -> rusqlite::Result<Vec<(String, Vec<f32>)>> {
        self.conn
            .prepare("SELECT fact_id,vector FROM fact_vectors WHERE model_id=?1")?
            .query_map([model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .filter_map(|row| match row {
                Ok((id, raw)) => memory::decode(&raw).map(|v| Ok((id, v))),
                Err(e) => Some(Err(e)),
            })
            .collect()
    }
    pub fn facts_without_vector(&self, model: &str, limit: usize) -> rusqlite::Result<Vec<Fact>> {
        self.conn
            .prepare("SELECT id,content,kind,privacy,origin,conversation_id,created_at,updated_at FROM facts WHERE NOT EXISTS(SELECT 1 FROM fact_vectors WHERE fact_id=facts.id AND model_id=?1) ORDER BY rowid LIMIT ?2")?
            .query_map(params![model, limit as i64], Self::fact_row)?
            .collect()
    }
    pub fn search_text(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<String>> {
        self.conn
            .prepare("SELECT facts.id FROM facts_fts JOIN facts ON facts.rowid=facts_fts.rowid WHERE facts_fts MATCH ?1 ORDER BY bm25(facts_fts) LIMIT ?2")?
            .query_map(params![query, limit as i64], |r| r.get(0))?
            .collect()
    }
    /// Conversa com mensagens completas ainda não processadas pela extração.
    pub fn pending_extraction(&self) -> rusqlite::Result<Option<(String, i64, i64)>> {
        self.conn
            .query_row("SELECT c.id, c.memory_cursor, MAX(m.id) FROM conversations c JOIN messages m ON m.conversation_id=c.id AND m.complete=1 GROUP BY c.id HAVING MAX(m.id)>c.memory_cursor ORDER BY MAX(m.id) LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })
    }
    pub fn turns_between(&self, conversation: &str, after: i64, until: i64, limit: usize) -> rusqlite::Result<Vec<Turn>> {
        let mut turns: Vec<Turn> = self.conn
            .prepare("SELECT role,content FROM messages WHERE conversation_id=?1 AND id>?2 AND id<=?3 AND complete=1 ORDER BY id DESC LIMIT ?4")?
            .query_map(params![conversation, after, until, limit as i64], |r| Ok(Turn { role: r.get(0)?, content: r.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
        turns.reverse();
        Ok(turns)
    }
    pub fn extraction_range(&self, conversation: &str) -> rusqlite::Result<Option<(i64, i64)>> {
        self.conn.query_row("SELECT c.memory_cursor, MAX(m.id) FROM conversations c JOIN messages m ON m.conversation_id=c.id AND m.complete=1 WHERE c.id=?1 GROUP BY c.id HAVING MAX(m.id)>c.memory_cursor", [conversation], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(Some)
            .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })
    }
    pub fn set_memory_cursor(&self, conversation: &str, cursor: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE conversations SET memory_cursor=MAX(memory_cursor,?2) WHERE id=?1", params![conversation, cursor])?;
        Ok(())
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
        assert_eq!(Store::open(&path).unwrap().version().unwrap(), 2);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn v1_database_is_backed_up_and_migrated_without_reprocessing_history() {
        let root = std::env::temp_dir().join(format!("ayla-v2-{}", id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ayla.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL, cloud_consent INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE messages(id INTEGER PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT NOT NULL, complete INTEGER NOT NULL);
                CREATE TABLE models(id TEXT PRIMARY KEY, definition TEXT NOT NULL);
                CREATE TABLE metrics(id INTEGER PRIMARY KEY, model_id TEXT NOT NULL, first_token_ms INTEGER, duration_ms INTEGER NOT NULL, tokens INTEGER NOT NULL, cancelled INTEGER NOT NULL);
                CREATE TABLE memories(id TEXT PRIMARY KEY, content TEXT NOT NULL, model_id TEXT NOT NULL, vector TEXT NOT NULL);
                INSERT INTO conversations(id,title) VALUES('c','antiga');
                INSERT INTO messages(conversation_id,role,content,complete) VALUES('c','user','moro em Fortaleza',1);
                INSERT INTO memories VALUES('n','Nota antiga sobre café','m','[0.1,0.2]');
                PRAGMA user_version=1;").unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.version().unwrap(), 2);
        let backups: Vec<_> = std::fs::read_dir(root.join("backups")).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        let old = Connection::open(backups[0].path()).unwrap();
        assert_eq!(old.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0)).unwrap(), 1);
        let facts = db.facts().unwrap();
        assert_eq!((facts.len(), facts[0].origin.as_str(), facts[0].privacy), (1, "manual", Privacy::Uncertain));
        // Vetores antigos não são reaproveitados; serão recalculados com o modelo atual.
        assert_eq!(db.facts_without_vector("m", 10).unwrap().len(), 1);
        assert!(db.pending_extraction().unwrap().is_none());
        db.add_message("c", "user", "lembre que tenho uma gata", true).unwrap();
        let (conversation, cursor, until) = db.pending_extraction().unwrap().unwrap();
        assert_eq!((conversation.as_str(), cursor), ("c", 1));
        assert_eq!(db.turns_between("c", cursor, until, 10).unwrap().len(), 1);
        db.set_memory_cursor("c", until).unwrap();
        db.set_memory_cursor("c", 0).unwrap();
        assert!(db.pending_extraction().unwrap().is_none() && db.extraction_range("c").unwrap().is_none());
        drop(db);
        assert_eq!(Store::open(&path).unwrap().version().unwrap(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn facts_support_editing_text_search_vectors_and_deletion() {
        let path = std::env::temp_dir().join(format!("ayla-facts-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let conversation = db.create_conversation("x").unwrap();
        let cat = db.insert_fact("O usuário tem uma gata chamada Mel.", "fato", Privacy::Uncertain, "auto", Some(&conversation.id)).unwrap();
        let rpg = db.insert_fact("O usuário mestra a campanha Cinzas de Aurora.", "projeto", Privacy::Sensitive, "manual", None).unwrap();
        db.set_vector(&cat.id, "emb", &[1.0, 0.0]).unwrap();
        db.set_vector(&rpg.id, "emb", &[0.0, 1.0]).unwrap();
        assert_eq!(db.vectors("emb").unwrap().len(), 2);
        assert_eq!(db.search_text(&memory::fts_query("Como se chama a mesa Aurora?").unwrap(), 5).unwrap(), vec![rpg.id.clone()]);
        // Acentos e caixa não impedem a busca textual.
        assert_eq!(db.search_text("\"cinzas\"", 5).unwrap().len(), 1);
        db.update_fact(&cat.id, Some("O usuário tem duas gatas."), Some(Privacy::Public)).unwrap();
        assert_eq!(db.vectors("emb").unwrap().len(), 1, "texto novo invalida o vetor");
        assert_eq!(db.facts_by_ids(&[cat.id.clone()]).unwrap()[0].privacy, Privacy::Public);
        assert!(db.search_text("\"gata\"", 5).unwrap().is_empty());
        assert_eq!(db.search_text("\"gatas\"", 5).unwrap(), vec![cat.id.clone()]);
        assert_eq!(db.conversation_facts(&conversation.id).unwrap().len(), 1);
        assert!(db.update_fact("inexistente", None, Some(Privacy::Public)).is_err());
        assert!(db.delete_fact(&rpg.id).unwrap() && !db.delete_fact(&rpg.id).unwrap());
        assert!(db.search_text("\"aurora\"", 5).unwrap().is_empty());
        assert_eq!(db.vectors("emb").unwrap().len(), 0, "apagar remove vetores em cascata");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}