"""Write the E2E fixture's WAV tracks into the folder given as the argument.

Each track is a quiet sine tone with a 1 kHz click on every beat, 20 s of
16-bit stereo at 44.1 kHz. The output is the same on every run.
"""

import math
import os
import struct
import sys
import wave

RATE = 44100
SECONDS = 20

TRACKS = [
    ("A440 120bpm.wav", 440.0, 120),
    ("C523 128bpm.wav", 523.25, 128),
    ("Slow/E330 90bpm.wav", 329.63, 90),
]


def write_track(path, tone_hz, bpm):
    beat = int(RATE * 60 / bpm)
    click = RATE // 100
    frames = bytearray()
    for i in range(RATE * SECONDS):
        v = 0.25 * math.sin(2 * math.pi * tone_hz * i / RATE)
        if i % beat < click:
            v += 0.6 * math.sin(2 * math.pi * 1000 * i / RATE)
        sample = int(32767 * v)
        frames += struct.pack("<hh", sample, sample)
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(bytes(frames))


def main():
    root = sys.argv[1]
    for name, tone_hz, bpm in TRACKS:
        path = os.path.join(root, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write_track(path, tone_hz, bpm)


if __name__ == "__main__":
    main()
