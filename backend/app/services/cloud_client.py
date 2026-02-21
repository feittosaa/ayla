import os
import json
import requests
from typing import Generator
from dotenv import load_dotenv

load_dotenv()

class CloudReadOnlyClient:
    def __init__(
        self,
        api_key=os.getenv("OPENAI_API_KEY"),
        model: str = "gpt-4o-mini",
        base_url: str = "https://api.openai.com/v1/chat/completions",
        timeout: int = 30,
    ):
        self.api_key = api_key or os.getenv("CLOUD_API_KEY")
        self.model = model
        self.base_url = base_url
        self.timeout = timeout

    def stream_read_only(self, prompt: str) -> Generator[str, None, None]:
        headers = {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }

        payload = {
            "model": self.model,
            "stream": True,
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "You are a stateless assistant. "
                        "You do not store, remember, or recall any information. "
                        "You answer only the current prompt."
                    ),
                },
                {
                    "role": "user",
                    "content": prompt,
                },
            ],
        }

        response = requests.post(
            self.base_url,
            headers=headers,
            json=payload,
            stream=True,
            timeout=self.timeout,
        )

        response.raise_for_status()

        for line in response.iter_lines():
            if not line:
                continue

            line = line.decode("utf-8")

            if not line.startswith("data:"):
                continue

            data = line.replace("data:", "").strip()

            if data == "[DONE]":
                break

            try:
                delta = (
                    json.loads(data)
                    .get("choices", [{}])[0]
                    .get("delta", {})
                    .get("content")
                )
                if delta:
                    yield delta
            except Exception:
                continue
