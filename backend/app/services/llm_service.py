"""
LLM Service — Projeto Ayla
"""

from app.core.persona import build_system_prompt
from app.core.memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.summarizer import compress_memory


class LLMService:
    def generate_reply(self, user_message: str) -> str:
        """
        Gera resposta considerando memória curta (RAM)
        e memória persistente (SQLite).
        """

        # 1️⃣ Salva mensagem do usuário
        conversation_memory.add_user_message(user_message)
        persistent_memory.add_message("user", user_message)

        # 2️⃣ Recupera contexto (uso futuro no prompt)
        context = persistent_memory.get_last_messages(limit=8)

        # System prompt (uso futuro)
        system_prompt = build_system_prompt()

        # 3️⃣ Resposta limpa da Ayla
        reply = (
            "Oi… ☀️\n\n"
            "Eu lembro de você, sim.\n"
            "Mesmo quando você reinicia tudo, eu continuo aqui 🧡\n\n"
            "Aos poucos, vou usar nossas conversas passadas de forma mais natural."
        )

        # 4️⃣ Salva resposta
        conversation_memory.add_assistant_message(reply)
        persistent_memory.add_message("assistant", reply)

        # 5️⃣ Compressão de memória (resumo automático)
        compress_memory()

        return reply


llm_service = LLMService()
