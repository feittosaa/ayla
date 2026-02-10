from app.core.chat_engine import handle_chat


def chat_ipc(user_message: str):
    """
    Entrada IPC (Tauri).
    """
    return handle_chat(user_message)
