"""
LLM Service — Projeto Ayla

Orquestra persona, memória, prompt e modelo de linguagem.
"""

from typing import Generator

from app.core.memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.prompt_builder import build_prompt
from app.services.ollama_client import OllamaClient


class LLMService:
    def __init__(self):
        self.ollama = OllamaClient(model="mistral")

    def generate_stream(self, user_message: str) -> Generator[str, None, None]:
        """
        Gera resposta da Ayla em streaming.
        """

        # 1️⃣ Salva mensagem do usuário
        conversation_memory.add_user_message(user_message)
        persistent_memory.add_message("user", user_message)

        # 2️⃣ Recupera contexto
        recent_messages = conversation_memory.get_messages()
        memory_summaries = (
            persistent_memory.get_all_summaries()
            if hasattr(persistent_memory, "get_all_summaries")
            else []
        )

        # 3️⃣ Monta prompt final
        prompt = build_prompt(
            user_message=user_message,
            recent_messages=recent_messages,
            memory_summaries=memory_summaries,
        )

        # 4️⃣ Envia para o Ollama (streaming)
        full_reply = ""

        for chunk in self.ollama.generate_stream(prompt):
            full_reply += chunk
            yield chunk

        # 5️⃣ Salva resposta da Ayla
        conversation_memory.add_assistant_message(full_reply)
        persistent_memory.add_message("assistant", full_reply)


# Instância única
llm_service = LLMService()
