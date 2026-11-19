#!/usr/bin/env python3
"""Regenerate the CC0 synthetic speech sample using eSpeak NG 1.51 (no microphone).

Needs libespeak-ng.so.1 and its data directory. The sentence is test data,
not a recording of a person or a sample from a service.
"""
import ctypes
from pathlib import Path
import wave

TEXT = b'This is a synthetic dictation test. The lighthouse keeper writes a new story every morning.'
engine = ctypes.CDLL('libespeak-ng.so.1')
rate = engine.espeak_Initialize(2, 0, None, 0)  # synchronous callback output
if rate <= 0:
    raise RuntimeError('eSpeak NG initialization failed')
frames = []
callback_type = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.POINTER(ctypes.c_short), ctypes.c_int, ctypes.c_void_p)

@callback_type
def collect(samples, count, events):
    if samples and count > 0:
        frames.append(ctypes.string_at(samples, count * 2))
    return 0

engine.espeak_SetSynthCallback(collect)
engine.espeak_SetVoiceByName(b'en-us')
engine.espeak_SetParameter(1, 140, 0)  # words/minute
engine.espeak_Synth(TEXT, len(TEXT) + 1, 0, 1, 0, 1, None, None)
engine.espeak_Synchronize()
engine.espeak_Terminate()
output = Path(__file__).resolve().parent.parent / 'fixtures' / 'synthetic-speech.wav'
with wave.open(str(output), 'wb') as wav:
    wav.setnchannels(1)
    wav.setsampwidth(2)
    wav.setframerate(rate)
    wav.writeframes(b''.join(frames))
print(output.name, rate, sum(map(len, frames)))
