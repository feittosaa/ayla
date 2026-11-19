"""Memória SQLite local, com caminho estável e conexões por operação."""

from contextlib import contextmanager
import os
from pathlib import Path
import sqlite3
import threading

BACKEND_DIR = Path(__file__).resolve().parents[2]


class PersistentMemory:
    def __init__(self, path=None):
        # Preserva o banco legado; importar a aplicação não abre nem altera dados.
        self.path = Path(path or os.getenv("AYLA_DB_PATH") or BACKEND_DIR / "ayla_memory.db").expanduser().resolve()
        self._initialized = False
        self._schema_lock = threading.Lock()

    @contextmanager
    def _connection(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        conn = sqlite3.connect(self.path, timeout=10)
        try:
            with self._schema_lock:
                if not self._initialized:
                    conn.executescript("""
                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE IF NOT EXISTS memory_summary (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    summary TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );
                    """)
                    self._initialized = True
            with conn:
                yield conn
        finally:
            conn.close()

    def add_message(self, role: str, content: str):
        with self._connection() as conn:
            conn.execute("INSERT INTO messages (role, content) VALUES (?, ?)", (role, content))

    def get_last_messages(self, limit: int = 10) -> list[dict[str, str]]:
        with self._connection() as conn:
            rows = conn.execute(
                "SELECT role, content FROM messages ORDER BY id DESC LIMIT ?", (limit,)
            ).fetchall()
        return [{"role": role, "content": content} for role, content in reversed(rows)]

    def add_summary(self, summary: str):
        with self._connection() as conn:
            conn.execute("INSERT INTO memory_summary (summary) VALUES (?)", (summary,))

    def get_all_summaries(self) -> list[str]:
        with self._connection() as conn:
            return [row[0] for row in conn.execute("SELECT summary FROM memory_summary ORDER BY id")]


persistent_memory = PersistentMemory()
