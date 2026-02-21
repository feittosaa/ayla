#llm_service.py

from app.services.ollama_client import OllamaClient
from app.services.inference_router import InferenceRouter
from app.services.cloud_client import CloudReadOnlyClient


class LLMService:
    def __init__(self):
        self.local_client = OllamaClient()

        self.cloud_client = CloudReadOnlyClient(
            model="gpt-4o-mini"  # ou outro
        )

        self.router = InferenceRouter(
            local_llm=self,
            cloud_llm=self.cloud_client,
        )

    def stream(self, local_prompt: str, cloud_prompt: str, private: bool):
        yield from self.router.stream(
            local_prompt=local_prompt,
            cloud_prompt=cloud_prompt,
            private=private,
        )

    def stream_local(self, prompt: str):
        yield from self.local_client.generate_stream(prompt)


llm_service = LLMService()

