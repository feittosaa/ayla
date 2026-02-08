"""
Ollama Client — Projeto Ayla
"""

import requests

OLLAMA_URL = "http://localhost:11434/api/generate"


def generate_llm_response(prompt: str) -> str:
    payload = {
        "model": "mistral",
        "prompt": prompt,
        "stream": False
    }

    response = requests.post(OLLAMA_URL, json=payload, timeout=180)
    response.raise_for_status()

    data = response.json()
    return data.get("response", "").strip()
