from fastapi import APIRouter, Depends
from fastapi.responses import StreamingResponse
from pydantic import BaseModel, Field, field_validator
from app.transports.http import chat_http
from app.core.security import verify_token
import json
from app.services.inference_router import ProviderUnavailable

router = APIRouter()

class ChatPayload(BaseModel):
    message: str = Field(min_length=1)

    @field_validator("message")
    @classmethod
    def reject_blank(cls, message):
        if not message.strip():
            raise ValueError("Mensagem não fornecida")
        return message


@router.post("/chat")
def chat(payload: ChatPayload, _=Depends(verify_token)):
    def event_generator():
        try:
            for chunk in chat_http(payload.message):
                # JSON preserva espaços, quebras de linha e caracteres do token.
                yield f"data: {json.dumps({'token': chunk}, ensure_ascii=False)}\n\n"
            yield 'event: done\ndata: {}\n\n'
        except ProviderUnavailable:
            yield 'event: error\ndata: {"message":"A IA está indisponível no momento."}\n\n'
        except Exception:
            yield 'event: error\ndata: {"message":"Não foi possível concluir a resposta."}\n\n'

    return StreamingResponse(
        event_generator(),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
    )
