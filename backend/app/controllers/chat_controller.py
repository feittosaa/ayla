"""
Chat Controller — Projeto Ayla

Endpoint de chat com streaming via Server-Sent Events (SSE).
"""

from fastapi import APIRouter, Depends
from fastapi.responses import StreamingResponse
from app.transports.http import chat_http, verify_token

router = APIRouter()


@router.post("/chat")
def chat(payload: dict, _=Depends(verify_token)):
    user_message = payload.get("message")

    if not user_message:
        return {"error": "Mensagem não fornecida"}

    def event_generator():
        for chunk in chat_http(user_message):
            yield f"data: {chunk}\n\n"

    return StreamingResponse(
        event_generator(),
        media_type="text/event-stream"
    )
