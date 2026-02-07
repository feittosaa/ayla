from fastapi import FastAPI
from app.controllers.chat_controller import router as chat_router

app = FastAPI(
    title="Projeto Ayla",
    description="Backend local da Ayla",
    version="0.1.0",
)

app.include_router(chat_router)


@app.get("/")
def health_check():
    return {"status": "Ayla está acordada 🌤️"}
