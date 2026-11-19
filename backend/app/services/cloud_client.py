"""Cliente cloud opcional. Não promete retenção que dependa do provedor."""

import json
import requests


class CloudReadOnlyClient:
    def __init__(self, api_key, model="gpt-4o-mini", timeout=30):
        self.api_key, self.model, self.timeout = api_key, model, timeout

    def stream_read_only(self, prompt):
        with requests.post(
            "https://api.openai.com/v1/chat/completions",
            headers={"Authorization": f"Bearer {self.api_key}", "Content-Type": "application/json"},
            json={"model": self.model, "stream": True, "messages": [{"role": "user", "content": prompt}]},
            stream=True,
            timeout=(5, self.timeout),
        ) as response:
            response.raise_for_status()
            for line in response.iter_lines():
                if not line or not line.startswith(b"data:"):
                    continue
                data = line[5:].strip()
                if data == b"[DONE]":
                    return
                event = json.loads(data)
                if event.get("error"):
                    raise RuntimeError("Provedor cloud não concluiu a resposta")
                choices = event.get("choices", [])
                if choices:
                    token = choices[0].get("delta", {}).get("content")
                    if token:
                        yield token
            raise RuntimeError("Stream cloud encerrado sem conclusão")
