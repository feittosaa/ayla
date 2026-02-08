"""
Prompt Builder — Projeto Ayla
"""

from app.core.persona import build_system_prompt
from app.core.memory import conversation_memory
from app.core.persistent_memory import persistent_memory


def build_prompt(user_message: str) -> str:
    system_prompt = build_system_prompt()

    short_memory = conversation_memory.get_context()
    summaries = persistent_memory.get_all_summaries()

    prompt = system_prompt + "\n\n"

    if summaries:
        prompt += "Memória importante sobre o usuário:\n"
        for s in summaries[-3:]:
            prompt += f"- {s}\n"
        prompt += "\n"

    prompt += "Conversa recente:\n"
    for msg in short_memory:
        role = "Usuário" if msg["role"] == "user" else "Ayla"
        prompt += f"{role}: {msg['content']}\n"

    prompt += f"\nUsuário: {user_message}\nAyla:"

    return prompt
