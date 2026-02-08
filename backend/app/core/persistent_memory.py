"""
Persistent Memory — Projeto Ayla

Memória persistente usando SQLite.
Armazena mensagens e resumos semânticos.
"""

import sqlite3
from pathlib import Path
from typing import List, Dict


DB_PATH = Path("ayla_memory.db")


class PersistentMemory:
    def __init__(self):
        self.conn = sqlite3.connect(DB_PATH, check_same_thread=False)
        self._init_db()

    def _init_db(self):
        cursor = self.conn.cursor()

        cursor.execute("""
        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
        """)

        cursor.execute("""
        CREATE TABLE IF NOT EXISTS memory_summary (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            summary TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
        """)

        self.conn.commit()

    # ---------- MENSAGENS ----------

    def add_message(self, role: str, content: str):
        cursor = self.conn.cursor()
        cursor.execute(
            "INSERT INTO messages (role, content) VALUES (?, ?)",
            (role, content)
        )
        self.conn.commit()

    def get_last_messages(self, limit: int = 10) -> List[Dict[str, str]]:
        cursor = self.conn.cursor()
        cursor.execute(
            "SELECT role, content FROM messages ORDER BY id DESC LIMIT ?",
            (limit,)
        )
        rows = cursor.fetchall()
        return [{"role": r[0], "content": r[1]} for r in reversed(rows)]

    # ---------- RESUMOS ----------

    def add_summary(self, summary: str):
        cursor = self.conn.cursor()
        cursor.execute(
            "INSERT INTO memory_summary (summary) VALUES (?)",
            (summary,)
        )
        self.conn.commit()

    def get_all_summaries(self) -> List[str]:
        cursor = self.conn.cursor()
        cursor.execute(
            "SELECT summary FROM memory_summary ORDER BY created_at ASC"
        )
        rows = cursor.fetchall()
        return [row[0] for row in rows]


# Singleton
persistent_memory = PersistentMemory()
