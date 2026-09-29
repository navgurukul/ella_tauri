# Resident Piper synthesis daemon. Loads the voice once, then serves
# newline-delimited JSON requests on stdin: {"text": "..."}.
# Each response is one JSON header line followed by exactly pcm_bytes of
# raw 16-bit mono PCM on stdout. Exits when stdin reaches EOF, so its
# lifetime is bound to the parent app.
#
# When the voice can say how long each token lasted, the header also carries
# "alignment": every token Piper spoke, in order, as [symbol, samples]. Ella's
# mouth follows it. The durations come from the same inference as the audio -
# Piper's duration predictor is random, so a second pass would not line up -
# which is why the voice is loaded with them exposed as a second output. That
# is done to the model's bytes in memory, rather than with the onnx package
# (68 MB) or a patched copy of the voice on disk, so any Piper voice works.
import json
import sys
import time


def send(header, payload=b""):
    sys.stdout.buffer.write((json.dumps(header) + "\n").encode("utf-8"))
    if payload:
        sys.stdout.buffer.write(payload)
    sys.stdout.buffer.flush()


# --- The model's bytes -------------------------------------------------------
# An ONNX file is one protobuf ModelProto. Only three of its fields matter
# here: the graph (ModelProto field 7), its nodes (GraphProto field 1, each
# naming its op type in field 4 and its outputs in field 2) and the graph's
# outputs (GraphProto field 12, each naming itself in field 1).


def _varint(data, at):
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, at
        shift += 7


def _encode_varint(value):
    out = bytearray()
    while value > 0x7F:
        out.append(value & 0x7F | 0x80)
        value >>= 7
    out.append(value)
    return bytes(out)


def _fields(data, start, end):
    """(number, wire type, body start, body end, field start) for each field."""
    at = start
    while at < end:
        field_start = at
        key, at = _varint(data, at)
        number, wire = key >> 3, key & 7
        if wire == 0:
            body = (at, _varint(data, at)[1])
        elif wire == 1:
            body = (at, at + 8)
        elif wire == 2:
            length, at = _varint(data, at)
            body = (at, at + length)
        elif wire == 5:
            body = (at, at + 4)
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        at = body[1]
        yield number, wire, body[0], body[1], field_start
    if at != end:
        raise ValueError("truncated protobuf message")


def _strings(data, start, end, number):
    return [bytes(data[s:e]) for n, w, s, e, _ in _fields(data, start, end) if n == number and w == 2]


def expose_durations(model):
    """The model with its sampled token durations as a second graph output.

    Piper's VITS graph ceils its predicted durations exactly once, into the
    frame counts the decoder renders; marking that tensor an output, as
    piper.patch_voice_with_alignment does, is all it takes. A model that
    already outputs it is returned as it is.
    """
    data = memoryview(model)
    graphs = [(at, s, e) for n, w, s, e, at in _fields(data, 0, len(data)) if n == 7 and w == 2]
    if len(graphs) != 1:
        raise ValueError("expected one graph in the voice model")
    field_start, start, end = graphs[0]
    ceils, outputs = [], set()
    for number, wire, s, e, _ in _fields(data, start, end):
        if wire != 2:
            continue
        if number == 1 and _strings(data, s, e, 4) == [b"Ceil"]:
            ceils.extend(_strings(data, s, e, 2))
        elif number == 12:
            outputs.update(_strings(data, s, e, 1))
    if len(ceils) != 1:
        raise ValueError(f"expected one Ceil in the voice model, found {len(ceils)}")
    if ceils[0] in outputs:
        return model
    value_info = b"\x0a" + _encode_varint(len(ceils[0])) + ceils[0]
    output = b"\x62" + _encode_varint(len(value_info)) + value_info
    graph = b"\x3a" + _encode_varint(end - start + len(output))
    return b"".join([data[:field_start], graph, data[start:end], output, data[end:]])


def load_voice(path):
    """The voice, and why it cannot report durations (None when it can)."""
    from piper import PiperVoice

    try:
        import onnxruntime
        from piper.config import PiperConfig

        with open(path, "rb") as handle:
            model = expose_durations(handle.read())
        with open(f"{path}.json", encoding="utf-8") as handle:
            config = PiperConfig.from_dict(json.load(handle))
        session = onnxruntime.InferenceSession(
            model,
            sess_options=onnxruntime.SessionOptions(),
            providers=["CPUExecutionProvider"],
        )
        return PiperVoice(config=config, session=session), None
    except Exception as error:  # noqa: BLE001 - any failure keeps the plain voice
        return PiperVoice.load(path), str(error)


def synthesize(voice, symbols, text):
    """PCM for the text, and each token's [symbol, samples] when every sentence
    reported them and they account for every sample of its audio."""
    try:
        chunks = voice.synthesize(text, include_alignments=symbols is not None)
    except TypeError:  # a piper-tts from before alignments
        chunks, symbols = voice.synthesize(text), None
    sample_rate = 22050
    parts = []
    alignment = [] if symbols is not None else None
    for chunk in chunks:
        sample_rate = chunk.sample_rate
        pcm = chunk.audio_int16_bytes
        parts.append(pcm)
        if alignment is None:
            continue
        ids = list(chunk.phoneme_ids)
        samples = getattr(chunk, "phoneme_id_samples", None)
        if (
            samples is None
            or len(samples) != len(ids)
            or int(samples.sum()) * 2 != len(pcm)
            or any(i not in symbols for i in ids)
        ):
            alignment = None
            continue
        alignment.extend([symbols[i], int(n)] for i, n in zip(ids, samples))
    return b"".join(parts), sample_rate, alignment


def token_symbols(voice):
    """Each phoneme id's symbol, from the voice's own phoneme map."""
    symbols = {}
    for symbol, ids in voice.config.phoneme_id_map.items():
        for i in ids:
            symbols.setdefault(i, symbol)
    return symbols


def main():
    try:
        started = time.time()
        voice, untimed = load_voice(sys.argv[1])
        symbols = token_symbols(voice) if untimed is None else None
        ready = {"ok": True, "ready_ms": int((time.time() - started) * 1000), "timed": untimed is None}
        if untimed is not None:
            ready["untimed"] = untimed
        send(ready)
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
            pcm, sample_rate, alignment = synthesize(voice, symbols, request["text"])
            if not pcm:
                send({"ok": False, "error": "piper produced no audio"})
                continue
            header = {
                "ok": True,
                "pcm_bytes": len(pcm),
                "sample_rate": sample_rate,
                "synth_ms": int((time.time() - started) * 1000),
            }
            if alignment is not None:
                header["alignment"] = alignment
            send(header, pcm)
        except Exception as error:  # noqa: BLE001 - keep serving after a bad request
            send({"ok": False, "error": str(error)})


if __name__ == "__main__":
    main()
