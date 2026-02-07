"""
Memory Module — Projeto Ayla

Gerencia a memória de curto prazo da Ayla (session).
Nada aqui é permanente ainda.
"""

from collections import deque
from typing import Deque, Dict, List


class ConversationMemory:
    def __init__(self, max_messages: int = 10):
        # Guarda as últimas mensagens da conversa
        self.max_messages = max_messages
        self.messages: Deque[Dict[str, str]] = deque(maxlen=max_messages)

    def add_user_message(self, message: str):
        self.messages.append({
            "role": "user",
            "content": message
        })

    def add_assistant_message(self, message: str):
        self.messages.append({
            "role": "assistant",
            "content": message
        })

    def get_context(self) -> List[Dict[str, str]]:
        """
        Retorna o histórico atual da conversa.
        """
        return list(self.messages)

    def clear(self):
        self.messages.clear()


# Memória única (session global por enquanto)
conversation_memory = ConversationMemory()
