"""Streaming local sem executor que retorna um gerador ainda não executado."""

import json
import requests


class OllamaClient:
    def __init__(self, model, base_url="http://127.0.0.1:11434", timeout=120):
        self.model, self.base_url, self.timeout = model, base_url.rstrip("/"), timeout

    def stream_local(self, prompt):
        with requests.post(
            f"{self.base_url}/api/generate",
            json={"model": self.model, "prompt": prompt, "stream": True},
            stream=True,
            timeout=(5, self.timeout),
        ) as response:
            response.raise_for_status()
            for line in response.iter_lines():
                if not line:
                    continue
                data = json.loads(line)
                if data.get("error"):
                    raise RuntimeError("Ollama não concluiu a resposta")
                if data.get("response"):
                    yield data["response"]
                if data.get("done"):
                    return
            raise RuntimeError("Stream local encerrado sem conclusão")

    generate_stream = stream_local
