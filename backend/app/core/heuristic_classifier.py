import re


class HeuristicClassifier:
    # Qualquer ocorrência → LOCAL obrigatório
    PRIVATE_KEYWORDS = [
        "meu", "minha", "meus", "minhas",
        "eu", "mim", "comigo",
        "lembra", "lembrar",
        "salva", "guarda",
        "memória", "memoria",
        "diário", "diario",
    ]

    PRIVATE_PATTERNS = [
        r"\bme lembra\b",
        r"\bguarda isso\b",
        r"\blembra disso\b",
        r"\bme conta sobre\b.*ontem",
    ]

    @classmethod
    def is_private(cls, text: str) -> bool:
        t = text.lower()

        for kw in cls.PRIVATE_KEYWORDS:
            if kw in t:
                return True

        for pattern in cls.PRIVATE_PATTERNS:
            if re.search(pattern, t):
                return True

        # ⚡ default = NÃO privado → cloud ok
        return False