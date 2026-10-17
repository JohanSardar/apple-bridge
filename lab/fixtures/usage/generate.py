from pathlib import Path
import struct
import zlib

root = Path(__file__).parent
base = 1800100800

def event(at, starting, bundle):
    encoded = bundle.encode("ascii")
    return bytes([16, 1, 24, starting, 33]) + struct.pack("<d", at - 978307200) + bytes([50, len(encoded)]) + encoded

def container(payloads):
    records = bytearray()
    trailer = bytearray()
    for index, payload in enumerate(payloads):
        records.extend(struct.pack("<II", zlib.crc32(payload), 0))
        records.extend(payload)
        trailer.extend(struct.pack("<iid", len(records), 1, base + index * 60 - 978307200))
        records.extend(bytes(-len(records) % 4))
    return struct.pack("<4sid16s", b"SEGB", len(payloads), base - 978307200, bytes(16)) + records + trailer

valid = [event(base, 1, "test.synthetic.alpha"), event(base + 60, 0, "test.synthetic.alpha")]
(root / "synthetic.segb").write_bytes(container(valid))
malformed = [event(float("nan"), 1, "test.synthetic.invalid"), event(base, 1, "invalid space"), event(base, 1, "test.synthetic.alpha")[:-4]]
(root / "malformed.segb").write_bytes(container(malformed + valid) + event(base, 1, "test.synthetic.truncated")[:-7])
