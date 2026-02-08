"""
Summarizer — Projeto Ayla

Responsável por condensar conversas antigas
em memória semântica.
"""

from app.core.persistent_memory import persistent_memory


def summarize_messages(messages: list[dict]) -> str:
    """
    Cria um resumo simples a partir de mensagens.
    (mock — depois vira LLM)
    """

    user_topics = []
    for msg in messages:
        if msg["role"] == "user":
            user_topics.append(msg["content"])

    if not user_topics:
        return ""

    summary = (
        "O usuário falou sobre os seguintes temas recentemente: "
        + "; ".join(user_topics[:5])
    )

    return summary


def compress_memory():
    """
    Resume mensagens antigas e limpa o excesso.
    """

    messages = persistent_memory.get_last_messages(limit=20)

    if len(messages) < 10:
        return  # nada pra resumir ainda

    summary = summarize_messages(messages)

    if summary:
        persistent_memory.add_summary(summary)
