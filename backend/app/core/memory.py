"""
Conversation Memory — Projeto Ayla

Memória de curto prazo (RAM).
Mantém apenas as últimas interações.
"""

from typing import List, Dict


class ConversationMemory:
    def __init__(self, max_messages: int = 10):
        self.max_messages = max_messages
        self._messages: List[Dict[str, str]] = []

    def add_user_message(self, content: str):
        self._add_message("user", content)

    def add_assistant_message(self, content: str):
        self._add_message("assistant", content)

    def _add_message(self, role: str, content: str):
        self._messages.append({
            "role": role,
            "content": content
        })

        # Mantém apenas as últimas mensagens
        if len(self._messages) > self.max_messages:
            self._messages = self._messages[-self.max_messages:]

    def get_messages(self) -> List[Dict[str, str]]:
        """
        Retorna o histórico curto da conversa.
        """
        return self._messages.copy()

    def clear(self):
        self._messages.clear()


# Instância única
conversation_memory = ConversationMemory()
