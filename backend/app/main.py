from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from app.controllers.chat_controller import router as chat_router
from fastapi import Depends
from app.core.security import verify_token
from app.services.llm_service import llm_service

app = FastAPI(
    title="Projeto Ayla",
    description="Backend local da Ayla",
    version="0.1.0",
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:5173", "http://127.0.0.1:5173", "tauri://localhost", "http://tauri.localhost"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(chat_router)


@app.get("/health")
def health(_=Depends(verify_token)):
    settings = llm_service.settings
    return {
        "status": "ok",
        "ai_configured": (
            (settings.local_enabled and bool(settings.local_model))
            or (settings.cloud_enabled and bool(settings.cloud_api_key))
        ),
    }
