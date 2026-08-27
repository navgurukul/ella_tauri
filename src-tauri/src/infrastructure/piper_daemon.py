# Resident Piper synthesis daemon. Loads the voice once, then serves
# newline-delimited JSON requests on stdin: {"text": "..."}.
# Each response is one JSON header line followed by exactly pcm_bytes of
# raw 16-bit mono PCM on stdout. Exits when stdin reaches EOF, so its
# lifetime is bound to the parent app.
import json
import sys
import time

MODEL_PATH = sys.argv[1]


def send(header, payload=b""):
    sys.stdout.buffer.write((json.dumps(header) + "\n").encode("utf-8"))
    if payload:
        sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()


try:
    started = time.time()
    from piper import PiperVoice

    voice = PiperVoice.load(MODEL_PATH)
    send({"ok": True, "ready_ms": int((time.time() - started) * 1000)})
except Exception as error:  # noqa: BLE001 - report every load failure to the app
    send({"ok": False, "error": f"piper voice load failed: {error}"})
    sys.exit(1)

for line in sys.stdin.buffer:
    line = line.strip()
    if not line:
        continue
    try:
        request = json.loads(line)
        started = time.time()
        sample_rate = 22050
        parts = []
        for chunk in voice.synthesize(request["text"]):
            sample_rate = chunk.sample_rate
            parts.append(chunk.audio_int16_bytes)
        pcm = b"".join(parts)
        if not pcm:
            send({"ok": False, "error": "piper produced no audio"})
            continue
        send(
            {
                "ok": True,
                "pcm_bytes": len(pcm),
                "sample_rate": sample_rate,
                "synth_ms": int((time.time() - started) * 1000),
            },
            pcm,
        )
    except Exception as error:  # noqa: BLE001 - keep serving after a bad request
        send({"ok": False, "error": str(error)})
