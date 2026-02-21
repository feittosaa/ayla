from app.services.circuit_breaker import CircuitBreaker


class InferenceRouter:
    def __init__(self, local_llm, cloud_llm=None):
        self.local = local_llm
        self.cloud = cloud_llm
        self.breaker = CircuitBreaker()

    def stream(self, local_prompt: str, cloud_prompt: str, private: bool):

        # 🔒 Privado = LOCAL SEM DISCUSSÃO
        if private:
            yield from self._local_only(local_prompt)
            return

        # ⚡ NÃO privado → CLOUD FIRST
        if self.cloud:
            try:
                yield from self.cloud.stream_read_only(cloud_prompt)
                return
            except Exception as e:
                pass

        # fallback local
        if self.breaker.allow():
            try:
                yield from self.local.stream_local(local_prompt)
                return
            except Exception:
                self.breaker.record_failure()

        yield "\n[Serviço temporariamente indisponível]\n"

    def _local_only(self, prompt):
        try:
            yield from self.local.stream_local(prompt)
        except Exception:
            self.breaker.record_failure()
            yield "\n[Ayla está indisponível no momento 💭]\n"

    def _sanitize_prompt(self, prompt: str) -> str:
      # remove possíveis blocos de memória
      forbidden = [
          "MEMORY:",
          "HISTÓRICO:",
          "CONTEXTO:",
          "RESUMOS:",
      ]

      for f in forbidden:
          prompt = prompt.replace(f, "")

      return prompt
