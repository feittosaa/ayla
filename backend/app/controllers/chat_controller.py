from fastapi import APIRouter
from pydantic import BaseModel

from app.core.persona import build_system_prompt

router = APIRouter(prefix="/chat", tags=["chat"])


class ChatRequest(BaseModel):
    message: str


class ChatResponse(BaseModel):
    reply: str


@router.post("", response_model=ChatResponse)
def chat_endpoint(request: ChatRequest):
    """
    Endpoint principal de conversa com a Ayla.
    Por enquanto, a resposta é mockada,
    mas já respeita a persona.
    """

    system_prompt = build_system_prompt()

    reply = (
        f"Oi… ☀️\n\n"
        f"Eu sou a Ayla.\n"
        f"Ainda estou aprendendo a falar direitinho, mas já consigo te ouvir.\n\n"
        f"Você disse:\n"
        f"\"{request.message}\"\n\n"
        f"(Essa é uma resposta inicial, sem IA real ainda 🧡)"
    )

    return ChatResponse(reply=reply)
