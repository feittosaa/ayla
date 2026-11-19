from app.core.config import Settings
from app.services.inference_router import InferenceRouter


class LLMService:
    def __init__(self, settings=None):
        self.settings = settings or Settings.from_env()
        self.router = None

    def _get_router(self):
        # Clientes leves, carregados apenas no primeiro uso da IA.
        if self.router is None:
            local = cloud = None
            if self.settings.local_enabled and self.settings.local_model:
                from app.services.ollama_client import OllamaClient
                local = OllamaClient(model=self.settings.local_model, base_url=self.settings.local_url)
            if self.settings.cloud_enabled and self.settings.cloud_api_key:
                from app.services.cloud_client import CloudReadOnlyClient
                cloud = CloudReadOnlyClient(api_key=self.settings.cloud_api_key, model=self.settings.cloud_model)
            self.router = InferenceRouter(local_llm=local, cloud_llm=cloud)
        return self.router

    def stream(self, local_prompt: str, cloud_prompt: str, private: bool):
        yield from self._get_router().stream(local_prompt, cloud_prompt, private)


llm_service = LLMService()
