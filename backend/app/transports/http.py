from fastapi import Header, HTTPException
from app.core.chat_engine import chat_stream
import os

APP_TOKEN = os.getenv("AYLA_TOKEN")


def verify_token(x_ayla_token: str = Header(None)):
    if x_ayla_token != APP_TOKEN:
        raise HTTPException(status_code=401)


def chat_http(user_message: str):
    """
    Transport HTTP (agnóstico de framework).
    """
    return chat_stream(user_message)
