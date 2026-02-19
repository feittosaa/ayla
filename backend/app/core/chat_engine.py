"""
Chat Engine — Projeto Ayla
Cérebro central da Ayla.
"""

from app.core.conversation_memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.prompt_builder import build_prompt
from app.core.memory_manager import maybe_compress_memory
from app.services.llm_service import llm_service


def chat_stream(user_message: str):
    # 1️⃣ salva mensagem do usuário
    conversation_memory.add_user_message(user_message)
    persistent_memory.add_message("user", user_message)

    # 2️⃣ recupera memórias
    recent_messages = conversation_memory.get_messages()
    summaries = persistent_memory.get_all_summaries()

    # 3️⃣ monta prompt final
    prompt = build_prompt(
        user_message=user_message,
        recent_messages=recent_messages,
        memory_summaries=summaries,
    )

    full_response = ""

    # 4️⃣ stream do modelo
    for chunk in llm_service.stream(prompt):
        full_response += chunk
        yield chunk

    # 5️⃣ salva resposta da Ayla
    conversation_memory.add_assistant_message(full_response)
    persistent_memory.add_message("assistant", full_response)

    # 6️⃣ compressão de memória
    maybe_compress_memory()
