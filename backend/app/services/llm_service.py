"""
LLM Service — Projeto Ayla

Orquestra persona, memória, prompt e modelo de linguagem.
"""

from app.services.ollama_client import OllamaClient
from app.core.memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.prompt_builder import build_prompt
from app.core.memory_manager import maybe_compress_memory

class LLMService:
    def __init__(self):
        self.client = OllamaClient()

    def generate_stream(self, user_message: str):
        # 1️⃣ salva mensagem do usuário
        conversation_memory.add_user_message(user_message)
        persistent_memory.add_message("user", user_message)

        # 2️⃣ recupera memórias
        recent_messages = conversation_memory.get_messages()
        summaries = persistent_memory.get_all_summaries()

        # 3️⃣ monta prompt completo
        prompt = build_prompt(
            user_message=user_message,
            recent_messages=recent_messages,
            memory_summaries=summaries,
        )

        # 4️⃣ stream do modelo
        full_response = ""

        for chunk in self.client.generate_stream(prompt):
            full_response += chunk
            yield chunk   # 👈 streaming continua intacto

        # 5️⃣ salva resposta da Ayla
        conversation_memory.add_assistant_message(full_response)
        persistent_memory.add_message("assistant", full_response)

        # 6️⃣ talvez resume memórias antigas
        maybe_compress_memory()


llm_service = LLMService()

