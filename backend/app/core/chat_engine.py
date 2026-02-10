from app.core.conversation_memory import conversation_memory
from app.core.persistent_memory import persistent_memory
from app.core.prompt_builder import build_prompt
from app.services.llm_service import llm_service


def handle_chat(user_message: str):
    """
    Núcleo do chat da Ayla.
    NÃO sabe se veio de HTTP ou IPC.
    """

    # memória curta
    conversation_memory.add_user_message(user_message)

    recent_messages = conversation_memory.get_messages()
    summaries = persistent_memory.get_all_summaries()

    prompt = build_prompt(
        user_message=user_message,
        recent_messages=recent_messages,
        memory_summaries=summaries,
    )

    for chunk in llm_service.generate_stream(prompt):
        yield chunk

    # ⚠️ quando terminar, salvar resposta completa
    # (isso pode ser refinado depois)
