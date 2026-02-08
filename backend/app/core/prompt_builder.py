"""
Prompt Builder — Projeto Ayla

Responsável por montar o prompt final enviado ao LLM,
com persona, memória e contexto recente.
"""

from typing import List, Dict
from app.core.persona import build_system_prompt


def build_prompt(
    user_message: str,
    recent_messages: List[Dict[str, str]],
    memory_summaries: List[str] | None = None,
) -> str:
    """
    Monta o prompt completo para o LLM.
    """

    sections = []

    # 1️⃣ SYSTEM — Persona da Ayla
    system_prompt = build_system_prompt()
    sections.append(system_prompt.strip())

    # 2️⃣ MEMÓRIA DE LONGO PRAZO (resumos)
    if memory_summaries:
        summaries_text = "\n".join(f"- {s}" for s in memory_summaries)
        sections.append(
            "MEMÓRIA IMPORTANTE SOBRE O USUÁRIO:\n"
            f"{summaries_text}"
        )

    # 3️⃣ CONTEXTO RECENTE (histórico curto)
    if recent_messages:
        conversation = []
        for msg in recent_messages:
            role = msg["role"].upper()
            content = msg["content"]
            conversation.append(f"{role}: {content}")

        sections.append(
            "CONVERSA RECENTE:\n" +
            "\n".join(conversation)
        )

    # 4️⃣ MENSAGEM ATUAL DO USUÁRIO
    sections.append(
        "USUÁRIO:\n"
        f"{user_message}\n\n"
        "AYLA:"
    )

    # Prompt final
    return "\n\n".join(sections)
