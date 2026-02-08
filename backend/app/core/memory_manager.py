import random
from app.core.summarizer import compress_memory

def maybe_compress_memory():
    # 10% de chance por mensagem
    if random.random() < 0.1:
        compress_memory()
