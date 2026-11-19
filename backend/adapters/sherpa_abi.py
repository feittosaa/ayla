"""ctypes ABI for sherpa-onnx v1.12.32 (Apache-2.0).
Source: https://github.com/k2-fsa/sherpa-onnx/blob/v1.12.32/sherpa-onnx/c-api/c-api.h
Generated from the pinned upstream header; update together with runtime artifact.
Header SHA256: 3ca64ce5af66d965fb2cd7cabbfc187d88808f4b65015c1439b47014825def25
"""
from ctypes import Structure, c_char_p, c_float, c_int32

class SherpaOnnxOfflineTtsVitsModelConfig(Structure):
    _fields_ = [
        ('model', c_char_p),
        ('lexicon', c_char_p),
        ('tokens', c_char_p),
        ('data_dir', c_char_p),
        ('noise_scale', c_float),
        ('noise_scale_w', c_float),
        ('length_scale', c_float),
        ('dict_dir', c_char_p),
    ]

class SherpaOnnxOfflineTtsMatchaModelConfig(Structure):
    _fields_ = [
        ('acoustic_model', c_char_p),
        ('vocoder', c_char_p),
        ('lexicon', c_char_p),
        ('tokens', c_char_p),
        ('data_dir', c_char_p),
        ('noise_scale', c_float),
        ('length_scale', c_float),
        ('dict_dir', c_char_p),
    ]

class SherpaOnnxOfflineTtsKokoroModelConfig(Structure):
    _fields_ = [
        ('model', c_char_p),
        ('voices', c_char_p),
        ('tokens', c_char_p),
        ('data_dir', c_char_p),
        ('length_scale', c_float),
        ('dict_dir', c_char_p),
        ('lexicon', c_char_p),
        ('lang', c_char_p),
    ]

class SherpaOnnxOfflineTtsKittenModelConfig(Structure):
    _fields_ = [
        ('model', c_char_p),
        ('voices', c_char_p),
        ('tokens', c_char_p),
        ('data_dir', c_char_p),
        ('length_scale', c_float),
    ]

class SherpaOnnxOfflineTtsZipvoiceModelConfig(Structure):
    _fields_ = [
        ('tokens', c_char_p),
        ('encoder', c_char_p),
        ('decoder', c_char_p),
        ('vocoder', c_char_p),
        ('data_dir', c_char_p),
        ('lexicon', c_char_p),
        ('feat_scale', c_float),
        ('t_shift', c_float),
        ('target_rms', c_float),
        ('guidance_scale', c_float),
    ]

class SherpaOnnxOfflineTtsPocketModelConfig(Structure):
    _fields_ = [
        ('lm_flow', c_char_p),
        ('lm_main', c_char_p),
        ('encoder', c_char_p),
        ('decoder', c_char_p),
        ('text_conditioner', c_char_p),
        ('vocab_json', c_char_p),
        ('token_scores_json', c_char_p),
        ('voice_embedding_cache_capacity', c_int32),
    ]

class SherpaOnnxOfflineTtsSupertonicModelConfig(Structure):
    _fields_ = [
        ('duration_predictor', c_char_p),
        ('text_encoder', c_char_p),
        ('vector_estimator', c_char_p),
        ('vocoder', c_char_p),
        ('tts_json', c_char_p),
        ('unicode_indexer', c_char_p),
        ('voice_style', c_char_p),
    ]

class SherpaOnnxOfflineTtsModelConfig(Structure):
    _fields_ = [
        ('vits', SherpaOnnxOfflineTtsVitsModelConfig),
        ('num_threads', c_int32),
        ('debug', c_int32),
        ('provider', c_char_p),
        ('matcha', SherpaOnnxOfflineTtsMatchaModelConfig),
        ('kokoro', SherpaOnnxOfflineTtsKokoroModelConfig),
        ('kitten', SherpaOnnxOfflineTtsKittenModelConfig),
        ('zipvoice', SherpaOnnxOfflineTtsZipvoiceModelConfig),
        ('pocket', SherpaOnnxOfflineTtsPocketModelConfig),
        ('supertonic', SherpaOnnxOfflineTtsSupertonicModelConfig),
    ]

class SherpaOnnxOfflineTtsConfig(Structure):
    _fields_ = [
        ('model', SherpaOnnxOfflineTtsModelConfig),
        ('rule_fsts', c_char_p),
        ('max_num_sentences', c_int32),
        ('rule_fars', c_char_p),
        ('silence_scale', c_float),
    ]
