from fastapi import APIRouter
from pydantic import BaseModel

from app.services.llm_service import llm_service

router = APIRouter(prefix="/chat", tags=["chat"])


class ChatRequest(BaseModel):
    message: str


class ChatResponse(BaseModel):
    reply: str


@router.post("", response_model=ChatResponse)
def chat_endpoint(request: ChatRequest):
    """
    Endpoint principal de conversa com a Ayla.
    O controller apenas delega a geração da resposta.
    """

    reply = llm_service.generate_reply(request.message)
    return ChatResponse(reply=reply)
