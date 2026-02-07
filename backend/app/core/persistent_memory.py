"""
Persistent Memory — Projeto Ayla

Memória persistente usando SQLite.
Armazena histórico básico de conversa.
"""

import sqlite3
from typing import List, Dict
from pathlib import Path


DB_PATH = Path("ayla_memory.db")


class PersistentConversationMemory:
    def __init__(self, db_path: Path = DB_PATH):
        self.db_path = db_path
        self._init_db()

    def _get_connection(self):
        return sqlite3.connect(self.db_path)

    def _init_db(self):
        with self._get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
                """
            )
            conn.commit()

    def add_message(self, role: str, content: str):
        with self._get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute(
                "INSERT INTO messages (role, content) VALUES (?, ?)",
                (role, content),
            )
            conn.commit()

    def get_last_messages(self, limit: int = 10) -> List[Dict[str, str]]:
        with self._get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute(
                """
                SELECT role, content
                FROM messages
                ORDER BY id DESC
                LIMIT ?
                """,
                (limit,),
            )
            rows = cursor.fetchall()

        # Retorna na ordem correta (mais antigo → mais novo)
        return [
            {"role": role, "content": content}
            for role, content in reversed(rows)
        ]

    def clear(self):
        with self._get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute("DELETE FROM messages")
            conn.commit()


# Instância única da memória persistente
persistent_memory = PersistentConversationMemory()
