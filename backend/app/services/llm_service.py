"""
LLM Service — Projeto Ayla

Este módulo é responsável por TODA interação com modelos de linguagem.
Nenhum controller deve falar diretamente com um modelo.

Por enquanto, este service é MOCKADO.
No futuro, aqui entram modelos locais, nuvem ou estratégia híbrida.
"""

from app.core.persona import build_system_prompt


class LLMService:
    def __init__(self):
        # No futuro:
        # - carregar modelo local
        # - configurar API externa
        # - decidir estratégia
        pass

    def generate_reply(self, user_message: str) -> str:
        """
        Gera uma resposta para o usuário.
        Por enquanto, retorna uma resposta mockada,
        mas já respeita a persona da Ayla.
        """

        system_prompt = build_system_prompt()

        # MOCK consciente
        reply = (
            "Oi… ☀️\n\n"
            "Eu sou a Ayla.\n"
            "Ainda estou no começo da minha jornada, mas já consigo conversar com você.\n\n"
            "Você me disse:\n"
            f"\"{user_message}\"\n\n"
            "Com o tempo, minhas respostas vão ficar cada vez mais inteligentes 🧡"
        )

        return reply


# Instância única do service (simples por enquanto)
llm_service = LLMService()
