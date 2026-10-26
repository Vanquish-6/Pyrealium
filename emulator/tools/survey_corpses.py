"""Find corpse CreateObjects (physics mode 41000003) in retained captures.

Usage: survey_corpses.py CAPTURE [CAPTURE ...]
Prints the corpse's visual, physics flags, entity and the messages that
followed it for the same object. Read-only.
"""
import struct
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from import_observed_population import messages

CREATE = 0x10058
CORPSE_MODE = 0x41000003


def main():
    for name in sys.argv[1:]:
        blobs = list(messages(Path(name).read_bytes()))
        corpses = {}
        for index, data in enumerate(blobs):
            if len(data) < 40 or struct.unpack_from("<I", data, 0)[0] != CREATE:
                continue
            if CORPSE_MODE.to_bytes(4, "little") not in data[:64]:
                continue
            object_id = struct.unpack_from("<Q", data, 4)[0]
            corpses[object_id] = index
            print(f"== {Path(name).name} corpse {object_id:016x} len {len(data)}")
            print("  head", data[:96].hex())
        follow = Counter()
        for index, data in enumerate(blobs):
            if len(data) >= 12:
                target = struct.unpack_from("<Q", data, 4)[0]
                if target in corpses and index != corpses[target]:
                    follow[f"{struct.unpack_from('<I', data, 0)[0]:#x}"] += 1
        print("  later opcodes on corpses:", follow.most_common(12))
        # Client requests that name a corpse anywhere in their payload.
        for index, data in enumerate(blobs):
            for object_id in corpses:
                if object_id.to_bytes(8, "little") in data[12:] and index != corpses[object_id]:
                    print(f"  ref {struct.unpack_from('<I', data, 0)[0]:#x} len {len(data)} {data[:72].hex()}")
                    break


if __name__ == "__main__":
    main()
