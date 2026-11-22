use super::files::Kind;
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
    pub project_id: Option<String>,
}
/// Espaço focado em algo específico, com conversas e memórias próprias.
#[derive(Clone, Debug, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// "Sobre o projeto": entra no contexto das respostas locais do projeto.
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub conversations: i64,
    pub memories: i64,
}
/// Quais memórias uma operação enxerga: as do projeto e, opcionalmente, as gerais.
/// Na conversa geral (`project: None`), só as gerais.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub project: Option<&'a str>,
    pub include_general: bool,
}
impl<'a> Scope<'a> {
    /// Respostas: o projeto vê também as memórias gerais.
    pub fn visible(project: Option<&'a str>) -> Self {
        Self { project, include_general: true }
    }
    /// Extração e painel: só o escopo exato; o que se aprende no projeto fica nele.
    pub fn exact(project: Option<&'a str>) -> Self {
        Self { project, include_general: false }
    }
}
/// Lembrete agendado.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    /// Próxima vez, em segundos Unix.
    pub due: i64,
    pub repeat: String,
    pub conversation_id: Option<String>,
}
/// Trecho de um arquivo guardado (Iris).
#[derive(Clone, Debug, Serialize)]
pub struct Chunk {
    pub id: i64,
    pub file_id: String,
    pub file_name: String,
    pub page: Option<u32>,
    pub text: String,
}
/// `?n,?n+1,…` para uma lista `IN` com `count` valores a partir do parâmetro `first`.
fn in_list(count: usize, first: usize) -> String {
    (first..first + count).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",")
}
const SCHEMA: u32 = 8;
const FACT_COLUMNS: &str = "id,content,kind,privacy,origin,conversation_id,created_at,updated_at,project_id";
const SCOPE_FILTER: &str = "(project_id IS ?S1 OR (?S2 AND project_id IS NULL))";
fn scoped(sql: &str) -> String {
    sql.replace("{scope}", SCOPE_FILTER)
}
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub complete: bool,
    pub origin: Option<String>,
    /// Arquivos enviados com esta mensagem.
    pub attachments: Vec<FileInfo>,
}
/// Arquivo guardado pela Ayla (cópia em `files/<id>/`).
#[derive(Clone, Debug, Serialize)]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub bytes: i64,
    pub created_at: i64,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub message_id: Option<i64>,
    /// Caminho relativo à pasta de dados.
    #[serde(skip)]
    pub path: String,
    #[serde(skip)]
    pub extracted: Option<String>,
}
const FILE_COLUMNS: &str = "id,name,kind,bytes,created_at,conversation_id,project_id,message_id,path,extracted";

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
        if version > SCHEMA {
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
        if (1..SCHEMA).contains(&version) {
            // Backup consistente antes de alterar um banco com dados do usuário.
            let backups = path.parent().unwrap_or(Path::new(".")).join("backups");
            std::fs::create_dir_all(&backups)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Self { conn }.backup(&backups.join(format!("ayla-antes-v{SCHEMA}-{}.db", id())))?;
            conn = Connection::open(path)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
        }
        if version <= 1 {
            Self::migrate_v2(&mut conn)?;
        }
        if version <= 2 {
            Self::migrate_v3(&mut conn)?;
        }
        if version <= 3 {
            Self::migrate_v4(&mut conn)?;
        }
        if version <= 4 {
            Self::migrate_v5(&mut conn)?;
        }
        if version <= 5 {
            Self::migrate_v6(&mut conn)?;
        }
        if version <= 6 {
            Self::migrate_v7(&mut conn)?;
        }
        if version <= 7 {
            let tx = conn.transaction()?;
            tx.execute_batch("ALTER TABLE messages ADD COLUMN origin TEXT; PRAGMA user_version=8;")?;
            tx.commit()?;
        }
        Ok(Self { conn })
    }
    /// v7 (Athena): índice das notas Markdown. O arquivo continua sendo a fonte de verdade; o
    /// índice guarda trechos (FTS5 e vetores) e a data/tamanho para reindexar só o que mudou.
    /// `note` é o caminho absoluto, então trocar de pasta não mistura notas.
    fn migrate_v7(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE note_index(note TEXT PRIMARY KEY, modified INTEGER NOT NULL, bytes INTEGER NOT NULL);
            CREATE TABLE note_chunks(id INTEGER PRIMARY KEY, note TEXT NOT NULL REFERENCES note_index(note) ON DELETE CASCADE, ord INTEGER NOT NULL, text TEXT NOT NULL);
            CREATE INDEX note_chunks_note ON note_chunks(note, ord);
            CREATE VIRTUAL TABLE note_chunks_fts USING fts5(text, content='note_chunks', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
            CREATE TRIGGER note_chunks_ai AFTER INSERT ON note_chunks BEGIN INSERT INTO note_chunks_fts(rowid,text) VALUES(new.id,new.text); END;
            CREATE TRIGGER note_chunks_ad AFTER DELETE ON note_chunks BEGIN INSERT INTO note_chunks_fts(note_chunks_fts,rowid,text) VALUES('delete',old.id,old.text); END;
            CREATE TABLE note_vectors(chunk_id INTEGER NOT NULL REFERENCES note_chunks(id) ON DELETE CASCADE, model_id TEXT NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(chunk_id, model_id));
            PRAGMA user_version=7;")?;
        tx.commit()
    }
    /// v6: lembretes. `due` em segundos Unix; `repeat` none/daily/weekly/monthly. Um lembrete
    /// sem repetição sai da tabela depois de avisar; os repetidos avançam para a próxima vez.
    fn migrate_v6(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE reminders(id TEXT PRIMARY KEY, text TEXT NOT NULL, due INTEGER NOT NULL, repeat TEXT NOT NULL,
                created_at INTEGER NOT NULL, conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL);
            CREATE INDEX reminders_due ON reminders(due);
            PRAGMA user_version=6;")?;
        tx.commit()
    }
    /// v5 (Iris): trechos dos arquivos guardados, com índice FTS5 e vetores por modelo, para
    /// responder com o que está nos arquivos do projeto e da conversa. `file_index` marca o que
    /// já foi dividido (inclusive arquivos sem texto). Apagar o arquivo apaga os trechos.
    fn migrate_v5(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE file_chunks(id INTEGER PRIMARY KEY, file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE, ord INTEGER NOT NULL, page INTEGER, text TEXT NOT NULL);
            CREATE INDEX file_chunks_file ON file_chunks(file_id, ord);
            CREATE VIRTUAL TABLE file_chunks_fts USING fts5(text, content='file_chunks', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
            CREATE TRIGGER file_chunks_ai AFTER INSERT ON file_chunks BEGIN INSERT INTO file_chunks_fts(rowid,text) VALUES(new.id,new.text); END;
            CREATE TRIGGER file_chunks_ad AFTER DELETE ON file_chunks BEGIN INSERT INTO file_chunks_fts(file_chunks_fts,rowid,text) VALUES('delete',old.id,old.text); END;
            CREATE TABLE chunk_vectors(chunk_id INTEGER NOT NULL REFERENCES file_chunks(id) ON DELETE CASCADE, model_id TEXT NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(chunk_id, model_id));
            CREATE TABLE file_index(file_id TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE, chunks INTEGER NOT NULL);
            PRAGMA user_version=5;")?;
        tx.commit()
    }
    /// v4: arquivos inseridos (cópias guardadas pela Ayla), ligados à conversa, ao
    /// projeto e à mensagem em que foram enviados; texto extraído fica em cache.
    /// `kept` marca o que já foi enviado ou adicionado em Arquivos; o resto é um
    /// anexo ainda na caixa de mensagem, descartado se nunca for enviado.
    fn migrate_v4(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE files(id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, bytes INTEGER NOT NULL, created_at INTEGER NOT NULL,
                conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE, project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL, path TEXT NOT NULL, extracted TEXT, sha256 TEXT NOT NULL,
                kept INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX files_conversation ON files(conversation_id);
            CREATE INDEX files_project ON files(project_id);
            CREATE INDEX files_message ON files(message_id);
            PRAGMA user_version=4;")?;
        tx.commit()
    }
    /// v3: projetos. Conversas e memórias ganham um projeto opcional; sem projeto,
    /// continuam gerais. Apagar um projeto apaga suas conversas e memórias.
    fn migrate_v3(conn: &mut Connection) -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            ALTER TABLE conversations ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE CASCADE;
            ALTER TABLE facts ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE CASCADE;
            CREATE INDEX conversations_project ON conversations(project_id);
            CREATE INDEX facts_project ON facts(project_id);
            PRAGMA user_version=3;")?;
        tx.commit()
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
    pub fn create_conversation(&self, title: &str, project: Option<&str>) -> rusqlite::Result<Conversation> {
        let conversation = Conversation {
            id: id(),
            title: title.chars().take(120).collect(),
            cloud_consent: false,
            project_id: project.map(Into::into),
        };
        self.conn.execute(
            "INSERT INTO conversations(id,title,project_id) VALUES(?1,?2,?3)",
            params![conversation.id, conversation.title, conversation.project_id],
        )?;
        Ok(conversation)
    }
    /// Conversas de um projeto, ou as gerais quando `project` é `None`.
    pub fn conversations(&self, project: Option<&str>) -> rusqlite::Result<Vec<Conversation>> {
        self.conn
            .prepare("SELECT id,title,cloud_consent,project_id FROM conversations WHERE project_id IS ?1 ORDER BY rowid DESC")?
            .query_map([project], |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    cloud_consent: r.get(2)?,
                    project_id: r.get(3)?,
                })
            })?
            .collect()
    }
    /// Título escolhido pela pessoa (1 a 120 caracteres).
    pub fn rename_conversation(&self, id: &str, title: &str) -> rusqlite::Result<bool> {
        let title: String = title.trim().chars().take(120).collect();
        Ok(self.conn.execute("UPDATE conversations SET title=?2 WHERE id=?1", params![id, title])? == 1)
    }
    /// Apaga a conversa, as mensagens e os arquivos dela; devolve os caminhos das cópias
    /// para apagar do disco. Memórias aprendidas nela ficam (sem o vínculo com a conversa).
    pub fn delete_conversation(&self, id: &str) -> rusqlite::Result<Option<Vec<String>>> {
        let tx = self.conn.unchecked_transaction()?;
        let paths: Vec<String> = tx.prepare("SELECT path FROM files WHERE conversation_id=?1")?.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        tx.execute("UPDATE facts SET conversation_id=NULL WHERE conversation_id=?1", [id])?;
        let deleted = tx.execute("DELETE FROM conversations WHERE id=?1", [id])? == 1;
        tx.commit()?;
        Ok(deleted.then_some(paths))
    }
    pub fn conversation_title(&self, conversation: &str) -> rusqlite::Result<String> {
        self.conn.query_row("SELECT title FROM conversations WHERE id=?1", [conversation], |r| r.get(0))
    }
    /// Renomeia só se o título ainda for `from` (não sobrescreve outra mudança).
    pub fn retitle(&self, conversation: &str, from: &str, to: &str) -> rusqlite::Result<bool> {
        let to: String = to.chars().take(120).collect();
        Ok(self.conn.execute("UPDATE conversations SET title=?3 WHERE id=?1 AND title=?2", params![conversation, from, to])? == 1)
    }
    pub fn conversation_project(&self, conversation: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT project_id FROM conversations WHERE id=?1", [conversation], |r| r.get(0))
    }
    pub fn create_project(&self, name: &str, description: &str) -> rusqlite::Result<Project> {
        let (id, now) = (id(), now());
        self.conn.execute(
            "INSERT INTO projects VALUES(?1,?2,?3,?4,?4)",
            params![id, name.trim().chars().take(80).collect::<String>(), description.trim().chars().take(2000).collect::<String>(), now],
        )?;
        self.project(&id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }
    pub fn update_project(&self, id: &str, name: &str, description: &str) -> rusqlite::Result<()> {
        let changed = self.conn.execute(
            "UPDATE projects SET name=?2, description=?3, updated_at=?4 WHERE id=?1",
            params![id, name.trim().chars().take(80).collect::<String>(), description.trim().chars().take(2000).collect::<String>(), now()],
        )?;
        if changed == 0 { Err(rusqlite::Error::QueryReturnedNoRows) } else { Ok(()) }
    }
    /// Apaga o projeto com suas conversas, mensagens e memórias.
    pub fn delete_project(&self, id: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM facts WHERE project_id=?1", [id])?;
        tx.execute("DELETE FROM conversations WHERE project_id=?1", [id])?;
        let deleted = tx.execute("DELETE FROM projects WHERE id=?1", [id])? == 1;
        tx.commit()?;
        Ok(deleted)
    }
    fn project_row(r: &rusqlite::Row) -> rusqlite::Result<Project> {
        Ok(Project { id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, created_at: r.get(3)?, updated_at: r.get(4)?, conversations: r.get(5)?, memories: r.get(6)? })
    }
    const PROJECT_SELECT: &'static str = "SELECT p.id,p.name,p.description,p.created_at,p.updated_at,(SELECT count(*) FROM conversations WHERE project_id=p.id),(SELECT count(*) FROM facts WHERE project_id=p.id) FROM projects p";
    pub fn project(&self, id: &str) -> rusqlite::Result<Option<Project>> {
        let mut rows = self.conn.prepare(&format!("{} WHERE p.id=?1", Self::PROJECT_SELECT))?.query_map([id], Self::project_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.pop())
    }
    pub fn projects(&self) -> rusqlite::Result<Vec<Project>> {
        self.conn.prepare(&format!("{} ORDER BY p.created_at, p.rowid", Self::PROJECT_SELECT))?.query_map([], Self::project_row)?.collect()
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
        let mut rows:Vec<Message> = self.conn.prepare("SELECT id,role,content,complete,origin FROM messages WHERE conversation_id=?1 ORDER BY id DESC LIMIT 200")?
            .query_map([id], |r| Ok(Message { id:r.get(0)?, role:r.get(1)?, content:r.get(2)?, complete:r.get(3)?, origin:r.get(4)?, attachments: Vec::new() }))?.collect::<rusqlite::Result<_>>()?;
        rows.reverse();
        let attached = self.conversation_files(id)?;
        for message in &mut rows {
            message.attachments = attached.iter().filter(|f| f.message_id == Some(message.id)).cloned().collect();
        }
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
    /// Grava a pergunta e a resposta; os anexos passam a pertencer à pergunta e os
    /// arquivos criados (ex.: imagens geradas), à resposta.
    pub fn save_turn(
        &self,
        id: &str,
        user: &str,
        assistant: &str,
        complete: bool,
        attachments: &[String],
        created: &[String],
        origin: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO messages(conversation_id,role,content,complete) VALUES(?1,'user',?2,1)",
            params![id, user],
        )?;
        let message = tx.last_insert_rowid();
        for file in attachments {
            tx.execute("UPDATE files SET conversation_id=?2, message_id=?3, kept=1 WHERE id=?1", params![file, id, message])?;
        }
        tx.execute("INSERT INTO messages(conversation_id,role,content,complete,origin) VALUES(?1,'assistant',?2,?3,?4)",params![id,assistant,complete,origin])?;
        let reply = tx.last_insert_rowid();
        for file in created {
            tx.execute("UPDATE files SET conversation_id=?2, message_id=?3, kept=1 WHERE id=?1", params![file, id, reply])?;
        }
        tx.commit()
    }
    fn file_row(r: &rusqlite::Row) -> rusqlite::Result<FileInfo> {
        let kind: String = r.get(2)?;
        Ok(FileInfo {
            id: r.get(0)?, name: r.get(1)?, kind: Kind::parse(&kind).unwrap_or(Kind::Text), bytes: r.get(3)?, created_at: r.get(4)?,
            conversation_id: r.get(5)?, project_id: r.get(6)?, message_id: r.get(7)?, path: r.get(8)?, extracted: r.get(9)?,
        })
    }
    /// `kept`: adicionado direto em Arquivos (fica); senão é anexo pendente da caixa de mensagem.
    pub fn insert_file(&self, file: &FileInfo, sha256: &str, kept: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            &format!("INSERT INTO files({FILE_COLUMNS},sha256,kept) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"),
            params![file.id, file.name, file.kind.as_str(), file.bytes, file.created_at, file.conversation_id, file.project_id, file.message_id, file.path, file.extracted, sha256, kept],
        )?;
        Ok(())
    }
    pub fn file(&self, id: &str) -> rusqlite::Result<Option<FileInfo>> {
        let mut rows = self.conn.prepare(&format!("SELECT {FILE_COLUMNS} FROM files WHERE id=?1"))?.query_map([id], Self::file_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.pop())
    }
    /// Arquivos já enviados numa conversa, do mais recente ao mais antigo.
    pub fn conversation_files(&self, conversation: &str) -> rusqlite::Result<Vec<FileInfo>> {
        self.conn.prepare(&format!("SELECT {FILE_COLUMNS} FROM files WHERE conversation_id=?1 AND message_id IS NOT NULL ORDER BY message_id DESC, rowid"))?.query_map([conversation], Self::file_row)?.collect()
    }
    /// Arquivos de um projeto (ou gerais), enviados ou adicionados direto em Arquivos.
    pub fn project_files(&self, project: Option<&str>) -> rusqlite::Result<Vec<FileInfo>> {
        self.conn.prepare(&format!("SELECT {FILE_COLUMNS} FROM files WHERE project_id IS ?1 AND kept=1 ORDER BY created_at DESC, rowid DESC"))?.query_map([project], Self::file_row)?.collect()
    }
    /// Athena: data e tamanho indexados de cada nota (caminho absoluto).
    pub fn note_stamps(&self) -> rusqlite::Result<std::collections::HashMap<String, (i64, i64)>> {
        self.conn.prepare("SELECT note,modified,bytes FROM note_index")?.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect()
    }
    pub fn index_note(&self, note: &str, modified: i64, bytes: i64, chunks: &[String]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM note_index WHERE note=?1", [note])?;
        tx.execute("INSERT INTO note_index VALUES(?1,?2,?3)", params![note, modified, bytes])?;
        for (ord, text) in chunks.iter().enumerate() {
            tx.execute("INSERT INTO note_chunks(note,ord,text) VALUES(?1,?2,?3)", params![note, ord as i64, text])?;
        }
        tx.commit()
    }
    pub fn forget_note(&self, note: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM note_index WHERE note=?1", [note])?;
        Ok(())
    }
    pub fn indexed_note_paths(&self, root: &str) -> rusqlite::Result<Vec<String>> {
        self.conn
            .prepare("SELECT note FROM note_index WHERE substr(note,1,length(?1))=?1")?
            .query_map([root], |r| r.get(0))?
            .collect()
    }
    /// Um trecho inicial por nota, para quando o tópico vem do nome da pasta.
    pub fn note_openings(&self, folder: &str, limit: usize) -> rusqlite::Result<Vec<i64>> {
        self.conn
            .prepare("SELECT coalesce(body.id,c.id) FROM note_index n
                JOIN note_chunks c ON c.note=n.note AND c.ord=0
                LEFT JOIN note_chunks body ON body.note=n.note AND body.ord=1 AND instr(c.text,char(10))=0
                WHERE substr(n.note,1,length(?1))=?1 ORDER BY n.modified DESC,n.note LIMIT ?2")?
            .query_map(params![folder, limit as i64], |r| r.get(0))?
            .collect()
    }
    /// Busca por palavras nas notas de uma pasta: (nota, trecho com destaque), uma linha por nota.
    pub fn search_notes(&self, query: &str, root: &str, limit: usize) -> rusqlite::Result<Vec<(String, String)>> {
        let mut seen = std::collections::HashSet::new();
        let rows: Vec<(String, String)> = self.conn
            .prepare("SELECT c.note, snippet(note_chunks_fts, 0, '«', '»', '…', 14) FROM note_chunks_fts JOIN note_chunks c ON c.id=note_chunks_fts.rowid WHERE note_chunks_fts MATCH ?1 AND substr(c.note,1,length(?2))=?2 ORDER BY bm25(note_chunks_fts) LIMIT 200")?
            .query_map(params![query, root], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().filter(|(note, _)| seen.insert(note.clone())).take(limit).collect())
    }
    /// Trechos com links wiki (`[[`) das notas de uma pasta, para os backlinks.
    pub fn notes_with_links(&self, root: &str) -> rusqlite::Result<Vec<(String, String)>> {
        self.conn
            .prepare("SELECT note,text FROM note_chunks WHERE substr(note,1,length(?1))=?1 AND instr(text,'[[')>0 ORDER BY note,ord")?
            .query_map([root], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }
    /// Trechos (id) que contêm as palavras, para a conversa.
    pub fn search_note_chunks(&self, query: &str, root: &str, limit: usize) -> rusqlite::Result<Vec<i64>> {
        self.conn
            .prepare(&format!("SELECT c.id FROM note_chunks_fts JOIN note_chunks c ON c.id=note_chunks_fts.rowid WHERE note_chunks_fts MATCH ?1 AND substr(c.note,1,length(?2))=?2 ORDER BY bm25(note_chunks_fts) LIMIT {limit}"))?
            .query_map(params![query, root], |r| r.get(0))?
            .collect()
    }
    pub fn note_chunks_without_vector(&self, model: &str, root: &str, limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        self.conn
            .prepare(&format!("SELECT id,text FROM note_chunks WHERE substr(note,1,length(?2))=?2 AND NOT EXISTS(SELECT 1 FROM note_vectors WHERE chunk_id=note_chunks.id AND model_id=?1) ORDER BY id LIMIT {limit}"))?
            .query_map(params![model, root], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }
    pub fn set_note_vector(&self, chunk: i64, model: &str, vector: &[f32]) -> rusqlite::Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO note_vectors VALUES(?1,?2,?3)", params![chunk, model, memory::encode(vector)])?;
        Ok(())
    }
    pub fn note_vectors(&self, model: &str, root: &str) -> rusqlite::Result<Vec<(i64, Vec<f32>)>> {
        self.conn
            .prepare("SELECT v.chunk_id,v.vector FROM note_vectors v JOIN note_chunks c ON c.id=v.chunk_id WHERE v.model_id=?1 AND substr(c.note,1,length(?2))=?2")?
            .query_map(params![model, root], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .filter_map(|row| match row {
                Ok((id, raw)) => memory::decode(&raw).map(|v| Ok((id, v))),
                Err(e) => Some(Err(e)),
            })
            .collect()
    }
    /// (nota, texto) dos trechos, na ordem pedida.
    pub fn note_chunks_by_ids(&self, ids: &[i64]) -> rusqlite::Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare("SELECT note,text FROM note_chunks WHERE id=?1")?;
        let mut out = Vec::new();
        for id in ids {
            if let Some(row) = statement.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.next() {
                out.push(row?);
            }
        }
        Ok(out)
    }
    pub fn add_reminder(&self, text: &str, due: i64, repeat: &str, conversation: Option<&str>) -> rusqlite::Result<Reminder> {
        let reminder = Reminder { id: id(), text: text.to_string(), due, repeat: repeat.to_string(), conversation_id: conversation.map(String::from) };
        self.conn.execute("INSERT INTO reminders VALUES(?1,?2,?3,?4,?5,?6)", params![reminder.id, reminder.text, reminder.due, reminder.repeat, now(), reminder.conversation_id])?;
        Ok(reminder)
    }
    /// Lembretes em ordem de horário; com `until`, só os que já venceram.
    pub fn reminders(&self, until: Option<i64>) -> rusqlite::Result<Vec<Reminder>> {
        self.conn
            .prepare("SELECT id,text,due,repeat,conversation_id FROM reminders WHERE ?1 IS NULL OR due<=?1 ORDER BY due, rowid")?
            .query_map([until], |r| Ok(Reminder { id: r.get(0)?, text: r.get(1)?, due: r.get(2)?, repeat: r.get(3)?, conversation_id: r.get(4)? }))?
            .collect()
    }
    pub fn reschedule_reminder(&self, id: &str, due: i64) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("UPDATE reminders SET due=?2 WHERE id=?1", params![id, due])? > 0)
    }
    pub fn delete_reminder(&self, id: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM reminders WHERE id=?1", [id])? > 0)
    }
    /// Iris: arquivo já dividido em trechos.
    pub fn indexed(&self, file: &str) -> rusqlite::Result<bool> {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM file_index WHERE file_id=?1)", [file], |r| r.get(0))
    }
    /// Troca os trechos de um arquivo (página opcional, texto).
    pub fn index_file(&self, file: &str, chunks: &[(Option<u32>, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM file_chunks WHERE file_id=?1", [file])?;
        for (ord, (page, text)) in chunks.iter().enumerate() {
            tx.execute("INSERT INTO file_chunks(file_id,ord,page,text) VALUES(?1,?2,?3,?4)", params![file, ord as i64, page, text])?;
        }
        tx.execute("INSERT OR REPLACE INTO file_index VALUES(?1,?2)", params![file, chunks.len() as i64])?;
        tx.commit()
    }
    /// Trechos que contêm as palavras da consulta FTS, dentro dos arquivos indicados.
    pub fn search_chunks(&self, query: &str, files: &[String], limit: usize) -> rusqlite::Result<Vec<i64>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!("SELECT c.id FROM file_chunks_fts JOIN file_chunks c ON c.id=file_chunks_fts.rowid WHERE file_chunks_fts MATCH ?1 AND c.file_id IN ({}) ORDER BY bm25(file_chunks_fts) LIMIT {limit}", in_list(files.len(), 2));
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&query];
        values.extend(files.iter().map(|f| f as &dyn rusqlite::ToSql));
        self.conn.prepare(&sql)?.query_map(values.as_slice(), |r| r.get(0))?.collect()
    }
    pub fn chunks_without_vector(&self, model: &str, files: &[String], limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!("SELECT id,text FROM file_chunks WHERE file_id IN ({}) AND NOT EXISTS(SELECT 1 FROM chunk_vectors WHERE chunk_id=file_chunks.id AND model_id=?1) ORDER BY id LIMIT {limit}", in_list(files.len(), 2));
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&model];
        values.extend(files.iter().map(|f| f as &dyn rusqlite::ToSql));
        self.conn.prepare(&sql)?.query_map(values.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }
    pub fn set_chunk_vector(&self, chunk: i64, model: &str, vector: &[f32]) -> rusqlite::Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO chunk_vectors VALUES(?1,?2,?3)", params![chunk, model, memory::encode(vector)])?;
        Ok(())
    }
    pub fn chunk_vectors(&self, model: &str, files: &[String]) -> rusqlite::Result<Vec<(i64, Vec<f32>)>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!("SELECT v.chunk_id,v.vector FROM chunk_vectors v JOIN file_chunks c ON c.id=v.chunk_id WHERE v.model_id=?1 AND c.file_id IN ({})", in_list(files.len(), 2));
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&model];
        values.extend(files.iter().map(|f| f as &dyn rusqlite::ToSql));
        self.conn.prepare(&sql)?
            .query_map(values.as_slice(), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .filter_map(|row| match row {
                Ok((id, raw)) => memory::decode(&raw).map(|v| Ok((id, v))),
                Err(e) => Some(Err(e)),
            })
            .collect()
    }
    /// Trechos na ordem pedida, com o nome do arquivo.
    pub fn chunks_by_ids(&self, ids: &[i64]) -> rusqlite::Result<Vec<Chunk>> {
        let mut out = Vec::new();
        let mut statement = self.conn.prepare("SELECT c.id,c.file_id,f.name,c.page,c.text FROM file_chunks c JOIN files f ON f.id=c.file_id WHERE c.id=?1")?;
        for id in ids {
            if let Some(chunk) = statement.query_map([id], |r| Ok(Chunk { id: r.get(0)?, file_id: r.get(1)?, file_name: r.get(2)?, page: r.get(3)?, text: r.get(4)? }))?.next() {
                out.push(chunk?);
            }
        }
        Ok(out)
    }
    pub fn set_extracted(&self, id: &str, text: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE files SET extracted=?2 WHERE id=?1", params![id, text])?;
        Ok(())
    }
    /// Remove o registro e devolve o caminho relativo para apagar a cópia.
    pub fn delete_file(&self, id: &str) -> rusqlite::Result<Option<String>> {
        let path: Option<String> = self.conn.query_row("SELECT path FROM files WHERE id=?1", [id], |r| r.get(0)).ok();
        self.conn.execute("DELETE FROM files WHERE id=?1", [id])?;
        Ok(path)
    }
    pub fn project_file_paths(&self, project: &str) -> rusqlite::Result<Vec<String>> {
        self.conn.prepare("SELECT path FROM files WHERE project_id=?1")?.query_map([project], |r| r.get(0))?.collect()
    }
    /// Caminhos de todos os arquivos registrados (para limpar cópias órfãs).
    pub fn file_paths(&self) -> rusqlite::Result<Vec<String>> {
        self.conn.prepare("SELECT path FROM files")?.query_map([], |r| r.get(0))?.collect()
    }
    /// Anexos escolhidos mas nunca enviados, mais antigos que `before`.
    pub fn forget_unsent_files(&self, before: i64) -> rusqlite::Result<usize> {
        self.conn.execute("DELETE FROM files WHERE kept=0 AND created_at<?1", [before])
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
        project: Option<&str>,
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
            project_id: project.map(Into::into),
        };
        self.conn.execute(
            &format!("INSERT INTO facts({FACT_COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8)"),
            params![fact.id, fact.content, fact.kind, privacy.as_str(), fact.origin, fact.conversation_id, now, fact.project_id],
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
            project_id: r.get(8)?,
        })
    }
    /// Memórias de um escopo exato (painel): as gerais ou as de um projeto.
    pub fn facts(&self, project: Option<&str>) -> rusqlite::Result<Vec<Fact>> {
        self.conn
            .prepare(&format!("SELECT {FACT_COLUMNS} FROM facts WHERE project_id IS ?1 ORDER BY updated_at DESC, rowid DESC LIMIT 5000"))?
            .query_map([project], Self::fact_row)?
            .collect()
    }
    pub fn facts_by_ids(&self, ids: &[String]) -> rusqlite::Result<Vec<Fact>> {
        let mut out = Vec::new();
        let mut statement = self.conn.prepare(&format!("SELECT {FACT_COLUMNS} FROM facts WHERE id=?1"))?;
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
            .prepare(&format!("SELECT {FACT_COLUMNS} FROM facts WHERE conversation_id=?1 AND origin<>'manual' ORDER BY rowid"))?
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
    pub fn vectors(&self, model: &str, scope: Scope) -> rusqlite::Result<Vec<(String, Vec<f32>)>> {
        self.conn
            .prepare(&scoped("SELECT fact_id,vector FROM fact_vectors JOIN facts ON facts.id=fact_vectors.fact_id WHERE model_id=?1 AND {scope}").replace("?S1", "?2").replace("?S2", "?3"))?
            .query_map(params![model, scope.project, scope.include_general], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .filter_map(|row| match row {
                Ok((id, raw)) => memory::decode(&raw).map(|v| Ok((id, v))),
                Err(e) => Some(Err(e)),
            })
            .collect()
    }
    pub fn facts_without_vector(&self, model: &str, limit: usize) -> rusqlite::Result<Vec<Fact>> {
        self.conn
            .prepare(&format!("SELECT {FACT_COLUMNS} FROM facts WHERE NOT EXISTS(SELECT 1 FROM fact_vectors WHERE fact_id=facts.id AND model_id=?1) ORDER BY rowid LIMIT ?2"))?
            .query_map(params![model, limit as i64], Self::fact_row)?
            .collect()
    }
    pub fn search_text(&self, query: &str, limit: usize, scope: Scope) -> rusqlite::Result<Vec<String>> {
        self.conn
            .prepare(&scoped("SELECT facts.id FROM facts_fts JOIN facts ON facts.rowid=facts_fts.rowid WHERE facts_fts MATCH ?1 AND {scope} ORDER BY bm25(facts_fts) LIMIT ?2").replace("?S1", "?3").replace("?S2", "?4"))?
            .query_map(params![query, limit as i64, scope.project, scope.include_general], |r| r.get(0))?
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
        let a = db.create_conversation("A", None).unwrap();
        let b = db.create_conversation("B", None).unwrap();
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
        assert_eq!(Store::open(&path).unwrap().version().unwrap(), SCHEMA);
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
        assert_eq!(db.version().unwrap(), SCHEMA);
        let backups: Vec<_> = std::fs::read_dir(root.join("backups")).unwrap().flatten().collect();
        assert_eq!(backups.len(), 1);
        let old = Connection::open(backups[0].path()).unwrap();
        assert_eq!(old.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0)).unwrap(), 1);
        let facts = db.facts(None).unwrap();
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
        assert_eq!(Store::open(&path).unwrap().version().unwrap(), SCHEMA);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn athena_indexa_busca_na_pasta_e_reindexa() {
        let path = std::env::temp_dir().join(format!("ayla-athena-idx-{}.db", id()));
        let db = Store::open(&path).unwrap();
        db.index_note("/notas/Treino.md", 10, 5, &["Agachamento e supino na segunda.".into()]).unwrap();
        db.index_note("/outra/Treino.md", 10, 5, &["Supino na outra pasta.".into()]).unwrap();
        let hits = db.search_notes("\"supino\"", "/notas/", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "/notas/Treino.md");
        assert!(hits[0].1.contains("«supino»"), "{}", hits[0].1);
        db.index_note("/notas/Treino.md", 20, 6, &["Corrida no sábado.".into()]).unwrap();
        assert!(db.search_notes("\"supino\"", "/notas/", 10).unwrap().is_empty(), "reindexada");
        assert_eq!(db.note_stamps().unwrap()["/notas/Treino.md"], (20, 6));
        let ids = db.search_note_chunks("\"corrida\"", "/notas/", 5).unwrap();
        db.set_note_vector(ids[0], "emb", &[1.0, 0.0]).unwrap();
        assert_eq!(db.note_vectors("emb", "/notas/").unwrap().len(), 1);
        assert_eq!(db.note_chunks_by_ids(&ids).unwrap()[0].1, "Corrida no sábado.");
        db.forget_note("/notas/Treino.md").unwrap();
        assert!(db.note_chunks_by_ids(&ids).unwrap().is_empty() && db.note_vectors("emb", "/notas/").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn lembretes_vencem_reagendam_e_saem() {
        let path = std::env::temp_dir().join(format!("ayla-lembretes-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let later = db.add_reminder("regar as plantas", 2_000, "daily", None).unwrap();
        let soon = db.add_reminder("ligar pro médico", 1_000, "none", None).unwrap();
        assert_eq!(db.reminders(None).unwrap().iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["ligar pro médico", "regar as plantas"]);
        assert_eq!(db.reminders(Some(1_500)).unwrap(), vec![soon.clone()]);
        assert!(db.reschedule_reminder(&later.id, 90_000).unwrap());
        assert!(db.delete_reminder(&soon.id).unwrap());
        assert!(db.reminders(Some(5_000)).unwrap().is_empty());
        assert_eq!(db.reminders(None).unwrap()[0].due, 90_000);
        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn iris_indexa_busca_no_escopo_e_apaga_com_o_arquivo() {
        let path = std::env::temp_dir().join(format!("ayla-iris-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let file = |id: &str, name: &str| FileInfo { id: id.into(), name: name.into(), kind: Kind::Pdf, bytes: 1, created_at: 0, conversation_id: None, project_id: None, message_id: None, path: format!("files/{id}/{name}"), extracted: None };
        db.insert_file(&file("a", "contrato.pdf"), "sha", true).unwrap();
        db.insert_file(&file("b", "outro.pdf"), "sha", true).unwrap();
        assert!(!db.indexed("a").unwrap());
        db.index_file("a", &[(Some(1), "Partes e objeto".into()), (Some(3), "O prazo de entrega é 12 de março.".into())]).unwrap();
        db.index_file("b", &[(None, "O prazo do outro documento é abril.".into())]).unwrap();
        db.index_file("b", &[]).unwrap();
        assert!(db.indexed("a").unwrap() && db.indexed("b").unwrap(), "sem texto também conta como indexado");
        let only_a = vec!["a".to_string()];
        let hits = db.search_chunks("\"prazo\"", &only_a, 5).unwrap();
        assert_eq!(hits.len(), 1);
        let chunk = &db.chunks_by_ids(&hits).unwrap()[0];
        assert_eq!((chunk.file_name.as_str(), chunk.page), ("contrato.pdf", Some(3)));
        assert!(db.search_chunks("\"prazo\"", &["b".to_string()], 5).unwrap().is_empty(), "trechos antigos substituídos");
        assert!(db.search_chunks("\"marco\"", &only_a, 5).unwrap().len() == 1, "sem acentos na busca");
        assert_eq!(db.chunks_without_vector("emb", &only_a, 10).unwrap().len(), 2);
        db.set_chunk_vector(chunk.id, "emb", &[1.0, 0.0]).unwrap();
        assert_eq!(db.chunk_vectors("emb", &only_a).unwrap().len(), 1);
        assert_eq!(db.chunks_without_vector("emb", &only_a, 10).unwrap().len(), 1);
        db.delete_file("a").unwrap();
        assert!(db.chunks_by_ids(&hits).unwrap().is_empty());
        assert!(db.search_chunks("\"prazo\"", &only_a, 5).unwrap().is_empty() && !db.indexed("a").unwrap());
        let _ = std::fs::remove_file(&path);
    }
    #[test]
    fn cloud_origin_survives_reopening_and_old_turns_remain_unlabelled() {
        let root = std::env::temp_dir().join(format!("ayla-origin-{}", id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("ayla.db");
        let db = Store::open(&path).unwrap();
        let chat = db.create_conversation("teste", None).unwrap();
        db.save_turn(&chat.id,"a","b",true,&[],&[],None).unwrap();
        db.conn.execute_batch("ALTER TABLE messages DROP COLUMN origin; PRAGMA user_version=7;").unwrap();
        drop(db);
        let db = Store::open(&path).unwrap();
        db.save_turn(&chat.id,"c","d",true,&[],&[],Some("cloud_search")).unwrap();
        drop(db);
        let rows = Store::open(&path).unwrap().messages(&chat.id).unwrap();
        assert!(rows[..3].iter().all(|row| row.origin.is_none()));
        assert_eq!(rows[3].origin.as_deref(),Some("cloud_search"));
        assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(),1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn facts_support_editing_text_search_vectors_and_deletion() {
        let path = std::env::temp_dir().join(format!("ayla-facts-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let conversation = db.create_conversation("x", None).unwrap();
        let cat = db.insert_fact("O usuário tem uma gata chamada Mel.", "fato", Privacy::Uncertain, "auto", Some(&conversation.id), None).unwrap();
        let rpg = db.insert_fact("O usuário mestra a campanha Cinzas de Aurora.", "projeto", Privacy::Sensitive, "manual", None, None).unwrap();
        db.set_vector(&cat.id, "emb", &[1.0, 0.0]).unwrap();
        db.set_vector(&rpg.id, "emb", &[0.0, 1.0]).unwrap();
        assert_eq!(db.vectors("emb", Scope::visible(None)).unwrap().len(), 2);
        assert_eq!(db.search_text(&memory::fts_query("Como se chama a mesa Aurora?").unwrap(), 5, Scope::visible(None)).unwrap(), vec![rpg.id.clone()]);
        // Acentos e caixa não impedem a busca textual.
        assert_eq!(db.search_text("\"cinzas\"", 5, Scope::visible(None)).unwrap().len(), 1);
        db.update_fact(&cat.id, Some("O usuário tem duas gatas."), Some(Privacy::Public)).unwrap();
        assert_eq!(db.vectors("emb", Scope::visible(None)).unwrap().len(), 1, "texto novo invalida o vetor");
        assert_eq!(db.facts_by_ids(&[cat.id.clone()]).unwrap()[0].privacy, Privacy::Public);
        assert!(db.search_text("\"gata\"", 5, Scope::visible(None)).unwrap().is_empty());
        assert_eq!(db.search_text("\"gatas\"", 5, Scope::visible(None)).unwrap(), vec![cat.id.clone()]);
        assert_eq!(db.conversation_facts(&conversation.id).unwrap().len(), 1);
        assert!(db.update_fact("inexistente", None, Some(Privacy::Public)).is_err());
        assert!(db.delete_fact(&rpg.id).unwrap() && !db.delete_fact(&rpg.id).unwrap());
        assert!(db.search_text("\"aurora\"", 5, Scope::visible(None)).unwrap().is_empty());
        assert_eq!(db.vectors("emb", Scope::visible(None)).unwrap().len(), 0, "apagar remove vetores em cascata");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn projects_keep_their_own_conversations_and_memories_and_see_general_ones() {
        let root = std::env::temp_dir().join(format!("ayla-projects-{}", id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ayla.db");
        {
            // Banco v2 com uma conversa e uma memória geral: a migração preserva os dois.
            let db = Store::open(&path).unwrap();
            let general = db.create_conversation("geral", None).unwrap();
            db.insert_fact("O usuário mora em Recife.", "fato", Privacy::Uncertain, "manual", None, None).unwrap();
            db.conn.execute_batch("ALTER TABLE messages DROP COLUMN origin; DROP TABLE note_vectors; DROP TABLE note_chunks_fts; DROP TABLE note_chunks; DROP TABLE note_index; DROP TABLE reminders; DROP TABLE file_index; DROP TABLE chunk_vectors; DROP TABLE file_chunks_fts; DROP TABLE file_chunks; DROP TABLE files; DROP INDEX conversations_project; DROP INDEX facts_project; ALTER TABLE conversations DROP COLUMN project_id; ALTER TABLE facts DROP COLUMN project_id; DROP TABLE projects; PRAGMA user_version=2;").unwrap();
            drop(general);
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.version().unwrap(), SCHEMA);
        assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 1, "backup antes de migrar");
        assert_eq!((db.conversations(None).unwrap().len(), db.facts(None).unwrap().len()), (1, 1));

        let tcc = db.create_project("  TCC  ", "Monografia sobre energia solar").unwrap();
        let casa = db.create_project("Mudança", "").unwrap();
        assert_eq!((tcc.name.as_str(), tcc.conversations, tcc.memories), ("TCC", 0, 0));
        let chat = db.create_conversation("Capítulo 1", Some(&tcc.id)).unwrap();
        assert_eq!(db.conversation_project(&chat.id).unwrap().as_deref(), Some(tcc.id.as_str()));
        let solar = db.insert_fact("O TCC usa dados do INMET.", "projeto", Privacy::Uncertain, "auto", Some(&chat.id), Some(&tcc.id)).unwrap();
        let caixa = db.insert_fact("A mudança é em março.", "projeto", Privacy::Uncertain, "manual", None, Some(&casa.id)).unwrap();
        let geral = db.facts(None).unwrap()[0].id.clone();
        for (fact, v) in [(&solar.id, [1.0, 0.0]), (&caixa.id, [0.0, 1.0]), (&geral, [0.5, 0.5])] {
            db.set_vector(fact, "emb", &v).unwrap();
        }
        let ids = |scope| { let mut v: Vec<String> = db.vectors("emb", scope).unwrap().into_iter().map(|(id, _)| id).collect(); v.sort(); v };
        let sorted = |mut v: Vec<String>| { v.sort(); v };
        // Conversa geral: só memórias gerais.
        assert_eq!(ids(Scope::visible(None)), vec![geral.clone()]);
        // Projeto: as dele e as gerais, nunca as de outro projeto.
        assert_eq!(ids(Scope::visible(Some(&tcc.id))), sorted(vec![solar.id.clone(), geral.clone()]));
        // Extração e painel: só o escopo exato.
        assert_eq!(ids(Scope::exact(Some(&tcc.id))), vec![solar.id.clone()]);
        assert!(db.search_text("\"março\"", 5, Scope::visible(Some(&tcc.id))).unwrap().is_empty());
        assert_eq!(db.search_text("\"março\"", 5, Scope::exact(Some(&casa.id))).unwrap(), vec![caixa.id.clone()]);
        assert_eq!((db.conversations(None).unwrap().len(), db.conversations(Some(&tcc.id)).unwrap().len()), (1, 1));
        assert_eq!(db.facts(Some(&tcc.id)).unwrap().len(), 1);
        let listed = db.projects().unwrap();
        assert_eq!(listed.iter().map(|p| (p.name.as_str(), p.conversations, p.memories)).collect::<Vec<_>>(), vec![("TCC", 1, 1), ("Mudança", 0, 1)]);

        db.update_project(&tcc.id, "TCC 2026", "Foco no capítulo de métodos").unwrap();
        assert_eq!(db.project(&tcc.id).unwrap().unwrap().description, "Foco no capítulo de métodos");
        assert!(db.update_project("inexistente", "x", "").is_err());
        // Apagar o projeto leva conversas, mensagens, memórias e vetores dele; o resto fica.
        db.add_message(&chat.id, "user", "rascunho", true).unwrap();
        assert!(db.delete_project(&tcc.id).unwrap());
        assert!(db.conversations(Some(&tcc.id)).unwrap().is_empty() && db.facts(Some(&tcc.id)).unwrap().is_empty());
        assert_eq!(db.conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(ids(Scope::visible(Some(&casa.id))), sorted(vec![caixa.id.clone(), geral.clone()]));
        assert_eq!((db.conversations(None).unwrap().len(), db.facts(None).unwrap().len()), (1, 1));
        assert!(!db.delete_project(&tcc.id).unwrap());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn conversations_can_be_renamed_and_deleted_keeping_memories() {
        let path = std::env::temp_dir().join(format!("ayla-delete-{}.db", id()));
        let db = Store::open(&path).unwrap();
        let (a, b) = (db.create_conversation("A", None).unwrap(), db.create_conversation("B", None).unwrap());
        assert!(db.rename_conversation(&a.id, "  Viagem de férias  ").unwrap());
        assert_eq!(db.conversation_title(&a.id).unwrap(), "Viagem de férias");
        assert!(!db.rename_conversation("inexistente", "x").unwrap());
        let file = FileInfo { id: "f1".into(), name: "nota.md".into(), kind: Kind::Text, bytes: 1, created_at: 0, conversation_id: None, project_id: None, message_id: None, path: "files/f1/nota.md".into(), extracted: None };
        db.insert_file(&file, "sha", false).unwrap();
        db.save_turn(&a.id, "oi", "olá", true, &[file.id.clone()], &[], None).unwrap();
        db.add_message(&b.id, "user", "fica", true).unwrap();
        let fact = db.insert_fact("O usuário vai viajar em julho.", "fato", Privacy::Uncertain, "auto", Some(&a.id), None).unwrap();
        assert_eq!(db.delete_conversation(&a.id).unwrap(), Some(vec!["files/f1/nota.md".to_string()]));
        assert!(db.messages(&a.id).unwrap().is_empty() && db.file("f1").unwrap().is_none());
        assert_eq!(db.conversations(None).unwrap().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), vec![b.id.clone()]);
        assert_eq!(db.messages(&b.id).unwrap().len(), 1, "outras conversas ficam");
        let kept = db.facts_by_ids(&[fact.id.clone()]).unwrap();
        assert_eq!((kept.len(), kept[0].conversation_id.clone()), (1, None), "a memória fica, sem o vínculo");
        assert_eq!(db.delete_conversation(&a.id).unwrap(), None);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
