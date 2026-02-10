from app.core.chat_engine import chat_stream


def chat_ipc(user_message: str):
    """
    Entrada IPC (Tauri).
    """
    return chat_stream(user_message)
