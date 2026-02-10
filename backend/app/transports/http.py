from fastapi import Header, HTTPException
from app.core.chat_engine import handle_chat
import os

APP_TOKEN = os.getenv("AYLA_TOKEN")


def verify_token(x_ayla_token: str = Header(None)):
    if x_ayla_token != APP_TOKEN:
        raise HTTPException(status_code=401)


def chat_http(user_message: str):
    return handle_chat(user_message)
