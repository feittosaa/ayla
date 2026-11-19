"""Adaptador isolado: texto chega por stdin, inferência nativa no sherpa-onnx.

Não abre rede, não persiste o pedido e não escreve conteúdo nos logs.
"""
import ctypes as c
import json
from pathlib import Path
import sys
from sherpa_abi import SherpaOnnxOfflineTtsConfig


class GeneratedAudio(c.Structure):
    _fields_ = [('samples', c.POINTER(c.c_float)), ('n', c.c_int32), ('sample_rate', c.c_int32)]


def synthesize(request):
    text = request.get('text', '')
    if not isinstance(text, str) or not text.strip() or len(text) > 20000:
        raise ValueError('Texto inválido')
    model = Path(request['model'])
    root = model.parent
    library = c.CDLL(request['library'])
    config = SherpaOnnxOfflineTtsConfig()
    config.model.vits.model = str(model).encode()
    config.model.vits.tokens = str(root / 'tokens.txt').encode()
    config.model.vits.lexicon = str(root / 'lexicon.txt').encode() if (root / 'lexicon.txt').is_file() else b''
    config.model.vits.data_dir = str(root / 'espeak-ng-data').encode() if (root / 'espeak-ng-data').is_dir() else b''
    config.model.vits.noise_scale = 0.667
    config.model.vits.noise_scale_w = 0.8
    config.model.vits.length_scale = 1.0
    config.model.num_threads = int(request.get('threads', 2))
    config.model.provider = b'cpu'
    config.max_num_sentences = 1
    config.silence_scale = 0.2
    library.SherpaOnnxCreateOfflineTts.argtypes = [c.POINTER(SherpaOnnxOfflineTtsConfig)]
    library.SherpaOnnxCreateOfflineTts.restype = c.c_void_p
    library.SherpaOnnxOfflineTtsGenerate.argtypes = [c.c_void_p, c.c_char_p, c.c_int32, c.c_float]
    library.SherpaOnnxOfflineTtsGenerate.restype = c.POINTER(GeneratedAudio)
    library.SherpaOnnxDestroyOfflineTts.argtypes = [c.c_void_p]
    library.SherpaOnnxDestroyOfflineTtsGeneratedAudio.argtypes = [c.POINTER(GeneratedAudio)]
    library.SherpaOnnxWriteWave.argtypes = [c.POINTER(c.c_float), c.c_int32, c.c_int32, c.c_char_p]
    library.SherpaOnnxWriteWave.restype = c.c_int32
    engine = library.SherpaOnnxCreateOfflineTts(c.byref(config))
    if not engine:
        raise RuntimeError('Modelo incompatível')
    audio = None
    try:
        audio = library.SherpaOnnxOfflineTtsGenerate(engine, text.encode(), 0, 1.0)
        if not audio or not 0 < audio.contents.n <= 32000000:
            raise RuntimeError('Áudio inválido')
        if not library.SherpaOnnxWriteWave(audio.contents.samples, audio.contents.n, audio.contents.sample_rate, str(request['output']).encode()):
            raise RuntimeError('Falha ao salvar áudio')
    finally:
        if audio:
            library.SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio)
        library.SherpaOnnxDestroyOfflineTts(engine)


if __name__ == '__main__':
    try:
        raw = sys.stdin.buffer.read(128 * 1024 + 1)
        if len(raw) > 128 * 1024:
            raise ValueError('Pedido muito grande')
        synthesize(json.loads(raw))
    except Exception:
        # Não imprimir textos, caminhos de usuário ou conteúdo de erros nativos.
        sys.exit(1)
