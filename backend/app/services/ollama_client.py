"""
Ollama Client

Cliente responsável por se comunicar com o Ollama
usando streaming puro (token por token).
"""

import json
import requests
from typing import Generator


import json
import requests
import concurrent.futures
from typing import Generator


class OllamaClient:
    def __init__(
        self,
        model: str = "mistral",
        base_url: str = "http://localhost:11434",
        timeout: int = 120,
    ):
        self.model = model
        self.base_url = base_url
        self.timeout = timeout
        self.executor = concurrent.futures.ThreadPoolExecutor(max_workers=1)

    def generate_stream(self, prompt: str) -> Generator[str, None, None]:
        future = self.executor.submit(self._stream_request, prompt)

        try:
            for token in future.result(timeout=self.timeout):
                yield token
        except Exception:
            future.cancel()
            raise

    def _stream_request(self, prompt: str):
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

            if "response" in data:
                yield data["response"]

            if data.get("done"):
                break
