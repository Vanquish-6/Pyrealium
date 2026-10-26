"""Survey captured CreateObjects inside a landblock range and why the importer
would reject them.

Usage: survey_landblock.py LANDBLOCK_HEX [RADIUS] CAPTURE_ROOT [CAPTURE_ROOT ...]
Example: survey_landblock.py 76b8 1 "...\\Packets"
Read-only.
"""
import struct
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from import_observed_population import Unsupported, messages, object_description


def main():
    block = int(sys.argv[1], 16)
    radius = int(sys.argv[2])
    roots = [Path(p) for p in sys.argv[3:]]
    reasons, visual_words, physics, files = Counter(), Counter(), Counter(), Counter()
    objects = {}
    for root in roots:
        for path in sorted(root.rglob("*")):
            if path.suffix.lower() not in {".pcap", ".elog"} or not path.is_file():
                continue
            try:
                blobs = list(messages(path.read_bytes()))
            except Exception:
                continue
            for data in blobs:
                if len(data) < 24 or data[:4] != b"\x58\x00\x01\x00":
                    continue
                identity = struct.unpack_from("<Q", data, 4)[0]
                # Find the cell by trying the plain layout; otherwise scan for the landblock.
                try:
                    actor = object_description(data)
                    cell = int(actor["cell"], 16)
                    reason = "accepted-shape"
                except Unsupported as error:
                    reason = str(error)
                    cell = None
                    for offset in range(20, min(len(data) - 4, 400), 4):
                        value = struct.unpack_from("<I", data, offset)[0]
                        if abs((value >> 24) - (block >> 8)) <= radius and abs(((value >> 16) & 255) - (block & 255)) <= radius and (value & 0xffff) <= 0x400:
                            cell = value
                            break
                if cell is None:
                    continue
                if abs((cell >> 24) - (block >> 8)) > radius or abs(((cell >> 16) & 255) - (block & 255)) > radius:
                    continue
                if identity >> 32 not in (0x81200000, 0x81300000) and identity >> 56 != 0x40:
                    continue
                objects.setdefault(identity, (reason, path.name, data))
    from ac2_dat import Dat
    game = Path(r"C:\Asheron's Call 2")
    with Dat(game / "portal.dat") as portal, Dat(game / "cell_1.dat") as cells:
        assets, cell_index = portal.index(), cells.index()
        for identity, (reason, name, data) in list(objects.items()):
            if reason != "accepted-shape":
                continue
            actor = object_description(data)
            entity, visual, cell = (int(actor[k], 16) for k in ("entity", "visual", "cell"))
            record = cell | 0xffff if (cell & 0xffff) <= 64 else cell
            missing = [label for label, ok in (("entity", entity in assets), ("visual", visual in assets), ("cell", record in cell_index)) if not ok]
            if missing:
                objects[identity] = ("missing-" + "+".join(missing), name, data)
    for identity, (reason, name, data) in objects.items():
        reasons[reason] += 1
        files[name] += 1
        visual_words[struct.unpack_from("<I", data, 12)[0]] += 1
    print(f"unique world objects near {block:04x}: {len(objects)}")
    print("by reason:", reasons.most_common())
    print("visual-desc first word:", [(f"{k:#x}", v) for k, v in visual_words.most_common(8)])
    print("captures:", files.most_common(12))
    samples = [v for v in objects.values() if v[0] == "unsupported-visual-description"][:3]
    for reason, name, data in samples:
        print(f"-- {name} len {len(data)}\n   {data[:160].hex()}")


if __name__ == "__main__":
    main()
