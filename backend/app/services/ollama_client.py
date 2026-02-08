"""
Ollama Client — Projeto Ayla

Cliente responsável por se comunicar com o Ollama
usando streaming puro (token por token).
"""

import json
import requests
from typing import Generator


class OllamaClient:
    def __init__(
        self,
        model: str = "mistral",
        base_url: str = "http://localhost:11434",
        timeout: int = 180,
    ):
        self.model = model
        self.base_url = base_url
        self.timeout = timeout

    def generate_stream(self, prompt: str) -> Generator[str, None, None]:
        """
        Gera resposta do LLM em streaming (chunk por chunk).
        Retorna um generator de strings.
        """

        response = requests.post(
            f"{self.base_url}/api/generate",
            json={
                "model": self.model,
                "prompt": prompt,
                "stream": True,
            },
            stream=True,
            timeout=self.timeout,
        )

        response.raise_for_status()

        for line in response.iter_lines():
            if not line:
                continue

            data = json.loads(line.decode("utf-8"))

            # Texto gerado pelo modelo
            if "response" in data:
                yield data["response"]

            # Fim da geração
            if data.get("done"):
                break
