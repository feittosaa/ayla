"""
LLM Service — Projeto Ayla
"""

from app.core.memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.summarizer import compress_memory
from app.core.prompt_builder import build_prompt
from app.services.ollama_client import generate_llm_response


class LLMService:
    def generate_reply(self, user_message: str) -> str:

        # 1️⃣ Salva mensagem do usuário
        conversation_memory.add_user_message(user_message)
        persistent_memory.add_message("user", user_message)

        # 2️⃣ Monta prompt com memória + persona
        prompt = build_prompt(user_message)

        # 3️⃣ Chama o LLM local
        reply = generate_llm_response(prompt)

        # 4️⃣ Salva resposta
        conversation_memory.add_assistant_message(reply)
        persistent_memory.add_message("assistant", reply)

        # 5️⃣ Resume memória se necessário
        compress_memory()

        return reply


llm_service = LLMService()
