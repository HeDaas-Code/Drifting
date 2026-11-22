#!/usr/bin/env python3
"""Recognize the testkit's synthetic speech with the packaged CPU runtime.

Only a public pinned model is downloaded. All audio stays in a temporary folder.
"""
from array import array
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import wave


def main():
    root = Path(__file__).resolve().parent.parent
    cache = root / '.cache' / 'whisper'
    runtime = cache / 'runtime' / ('whisper-cli.exe' if os.name == 'nt' else 'whisper-cli')
    model = cache / 'ggml-tiny.bin'
    # Read the same immutable pins used by the application downloader.
    pins = (root / 'crates/dr-platform/src/dictation/models.rs').read_text()
    revision = re.search(r'REVISION: &str = "([a-f0-9]{40})"', pins)[1]
    expected = re.search(r'file: "ggml-tiny.bin".*?sha256: "([a-f0-9]{64})"', pins, re.S)[1]
    if not model.exists() or hashlib.sha256(model.read_bytes()).hexdigest() != expected:
        partial = model.with_suffix('.partial')
        try:
            with urllib.request.urlopen(f'https://huggingface.co/ggerganov/whisper.cpp/resolve/{revision}/ggml-tiny.bin', timeout=60) as response, partial.open('wb') as out:
                shutil.copyfileobj(response, out)
            if hashlib.sha256(partial.read_bytes()).hexdigest() != expected:
                raise RuntimeError('Speech model integrity check failed')
            partial.replace(model)
        finally:
            partial.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix='dr-dictation-') as tmp:
        tmp = Path(tmp)
        with wave.open(str(root / 'crates/dr-testkit/fixtures/synthetic-speech.wav')) as source:
            assert source.getnchannels() == 1 and source.getsampwidth() == 2
            rate = source.getframerate()
            samples = array('h', source.readframes(source.getnframes()))
        if sys.byteorder != 'little':
            samples.byteswap()
        converted = array('h')
        for i in range(len(samples) * 16000 // rate):
            index, fraction = divmod(i * rate, 16000)
            left = samples[index]
            right = samples[min(index + 1, len(samples) - 1)]
            converted.append(left + int((right - left) * fraction / 16000))
        if sys.byteorder != 'little':
            converted.byteswap()
        wav = tmp / 'voice.wav'
        with wave.open(str(wav), 'wb') as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(16000)
            out.writeframes(converted.tobytes())
        subprocess.run([str(runtime), '--no-gpu', '-t', '4', '-l', 'auto', '-m', str(model), '-f', str(wav), '-otxt', '-of', str(tmp / 'result')], check=True, timeout=120, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        text = (tmp / 'result.txt').read_text().lower()
        for word in ['synthetic', 'dictation', 'lighthouse', 'keeper', 'story', 'morning']:
            if word not in text:
                raise AssertionError(f'Missing {word!r} in synthetic transcription: {text!r}')
        environment = os.environ.copy()
        environment['DRIFTING_WHISPER_CLI'] = str(runtime)
        environment['DRIFTING_WHISPER_MODEL'] = str(model)
        subprocess.run(['cargo', 'test', '--locked', '--workspace', '--lib', 'dictation::tests::cpu_whisper', '--', '--ignored'], cwd=root, env=environment, check=True)
        print('CPU dictation: pinned multilingual Tiny recognized all six synthetic keywords')


if __name__ == '__main__':
    main()
