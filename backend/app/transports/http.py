from app.core.chat_engine import chat_stream


def chat_http(user_message: str):
    """
    Transport HTTP (agnóstico de framework).
    """
    return chat_stream(user_message)
