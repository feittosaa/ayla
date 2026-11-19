"""Configuração local explícita; nenhum acesso à rede no carregamento."""

from dataclasses import dataclass
import os


@dataclass(frozen=True)
class Settings:
    local_enabled: bool
    cloud_enabled: bool
    local_model: str
    local_url: str
    cloud_model: str
    cloud_api_key: str

    @classmethod
    def from_env(cls):
        return cls(
            local_enabled=os.getenv("AYLA_LOCAL_ENABLED", "true").lower() == "true",
            cloud_enabled=os.getenv("AYLA_CLOUD_ENABLED", "false").lower() == "true",
            local_model=os.getenv("AYLA_LOCAL_MODEL", ""),
            local_url=os.getenv("AYLA_LOCAL_URL", "http://127.0.0.1:11434"),
            cloud_model=os.getenv("AYLA_CLOUD_MODEL", "gpt-4o-mini"),
            cloud_api_key=os.getenv("OPENAI_API_KEY") or os.getenv("CLOUD_API_KEY", ""),
        )
