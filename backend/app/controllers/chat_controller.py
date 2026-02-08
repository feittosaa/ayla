"""
Chat Controller — Projeto Ayla

Endpoint de chat com streaming via Server-Sent Events (SSE).
Envia texto ACUMULADO (não tokens crus).
"""

from fastapi import APIRouter
from fastapi.responses import StreamingResponse

from app.services.llm_service import llm_service

router = APIRouter()

@router.post("/chat")
def chat(payload: dict):
    user_message = payload.get("message")

    if not user_message:
        return {"error": "Mensagem não fornecida"}

    def event_generator():
        for chunk in llm_service.generate_stream(user_message):
            yield f"data: {chunk}\n\n"

    return StreamingResponse(
        event_generator(),
        media_type="text/event-stream"
    )
