from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from app.controllers.chat_controller import router as chat_router

app = FastAPI(
    title="Projeto Ayla",
    description="Backend local da Ayla",
    version="0.1.0",
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],  # só no dev
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(chat_router)
