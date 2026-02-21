from app.services.circuit_breaker import CircuitBreaker


class InferenceRouter:
    def __init__(self, local_llm, cloud_llm=None):
        self.local = local_llm
        self.cloud = cloud_llm
        self.breaker = CircuitBreaker()

    def stream(self, prompt: str, private: bool = True):
        # 🔒 Privado NUNCA usa cloud
        if private:
            yield from self._local_only(prompt)
            return

        # tenta local primeiro
        if self.breaker.allow():
            try:
                yield from self.local.stream_local(prompt)
                self.breaker.reset()
                return
            except Exception:
                self.breaker.record_failure()

        # ☁️ fallback cloud (read-only)
        if self.cloud:
            yield from self.cloud.stream_read_only(prompt)
        else:
            yield "\n[Serviço temporariamente indisponível]\n"

    def _local_only(self, prompt):
        try:
            yield from self.local.stream_local(prompt)
        except Exception:
            self.breaker.record_failure()
            yield "\n[Ayla está indisponível no momento 💭]\n"
