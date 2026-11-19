from app.services.circuit_breaker import CircuitBreaker


class ProviderUnavailable(RuntimeError):
    pass


class InferenceRouter:
    def __init__(self, local_llm=None, cloud_llm=None):
        self.local = local_llm
        self.cloud = cloud_llm
        self.breaker = CircuitBreaker()

    def stream(self, local_prompt: str, cloud_prompt: str, private: bool):
        # Local é o padrão. Falha após o primeiro token nunca reinicia em outro
        # provedor: isso misturaria respostas e poderia reenviar dados sem clareza.
        if self.local and self.breaker.allow():
            emitted = False
            try:
                for chunk in self.local.stream_local(local_prompt):
                    if chunk:
                        emitted = True
                        yield chunk
                if not emitted:
                    raise ProviderUnavailable("Resposta local vazia")
                self.breaker.reset()
                return
            except Exception as exc:
                self.breaker.record_failure()
                if emitted:
                    raise ProviderUnavailable("Resposta local interrompida") from exc

        # A heurística apenas restringe o envio; habilitar cloud exige decisão
        # explícita via configuração e não envia histórico nem resumos.
        if not private and self.cloud:
            emitted = False
            try:
                for chunk in self.cloud.stream_read_only(cloud_prompt):
                    if chunk:
                        emitted = True
                        yield chunk
                if not emitted:
                    raise ProviderUnavailable("Resposta cloud vazia")
                return
            except Exception as exc:
                raise ProviderUnavailable("Provedor cloud indisponível") from exc

        raise ProviderUnavailable("Nenhum provedor disponível para esta mensagem")
