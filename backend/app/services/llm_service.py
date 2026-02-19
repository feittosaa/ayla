from app.services.ollama_client import OllamaClient


class LLMService:
    def __init__(self):
        self.client = OllamaClient()

    def stream(self, prompt: str):
        """
        Recebe prompt pronto e devolve tokens.
        Não sabe nada sobre memória ou persona.
        """
        yield from self.client.generate_stream(prompt)


llm_service = LLMService()
