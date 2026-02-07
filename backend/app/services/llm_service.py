"""
LLM Service — Projeto Ayla
"""

from app.core.persona import build_system_prompt
from app.core.memory import conversation_memory


class LLMService:
    def generate_reply(self, user_message: str) -> str:
        """
        Gera resposta considerando a memória de curto prazo.
        """

        # Adiciona mensagem do usuário à memória
        conversation_memory.add_user_message(user_message)

        system_prompt = build_system_prompt()
        context = conversation_memory.get_context()

        # MOCK consciente usando contexto
        reply = (
            "Oi… ☀️\n\n"
            "Eu estou começando a lembrar das nossas mensagens.\n\n"
            "Até agora, nossa conversa tem sido:\n"
        )

        for msg in context:
            reply += f"- {msg['role']}: {msg['content']}\n"

        reply += (
            "\nCom o tempo, eu vou usar isso pra responder de forma mais natural 🧡"
        )

        # Adiciona resposta da Ayla à memória
        conversation_memory.add_assistant_message(reply)

        return reply


llm_service = LLMService()
