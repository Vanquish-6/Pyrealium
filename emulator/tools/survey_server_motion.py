"""Survey retained captures for server position updates (0x10086).

Usage: survey_server_motion.py CAPTURE_DIR [LIMIT]
Prints motion-word, contact-flag and step-interval statistics so moving
creature updates can follow observed retail values. Read-only.
"""
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from import_observed_population import messages

SERVER_POSITION = 0x10086


def main():
    root = Path(sys.argv[1])
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else 200
    motions, flags, by_object = Counter(), Counter(), defaultdict(list)
    files = 0
    for path in sorted(root.rglob("*")):
        if path.suffix.lower() not in {".pcap", ".elog"} or not path.is_file() or files >= limit:
            continue
        try:
            raw = path.read_bytes()
            for blob in messages(raw):
                if len(blob) < 56 or struct.unpack_from("<I", blob, 0)[0] != SERVER_POSITION:
                    continue
                object_id = struct.unpack_from("<Q", blob, 4)[0]
                time = struct.unpack_from("<d", blob, 16)[0]
                cell = struct.unpack_from("<I", blob, 24)[0]
                x, y, z = struct.unpack_from("<3f", blob, 28)
                motion, flag = struct.unpack_from("<2I", blob, 40)
                stamps = struct.unpack_from("<3H", blob, 48)
                motions[motion & 0xFFFFFF] += 1
                flags[flag] += 1
                by_object[(path.name, object_id)].append((time, cell, x, y, z, motion, stamps))
            files += 1
        except Exception:
            continue
    print(f"files {files} updates {sum(motions.values())}")
    print("motion low 24 bits:", [(f"{m:06x}", n) for m, n in motions.most_common(12)])
    print("flags:", [(f"{f:#x}", n) for f, n in flags.most_common(6)])
    prefixes = Counter(k[1] >> 48 for k in by_object)
    print("object prefixes:", [(f"{p:04x}", n) for p, n in prefixes.most_common(8)])
    npc = [(k, v) for k, v in by_object.items() if k[1] >> 48 != 0x8120 and len(v) >= 6]
    npc.sort(key=lambda kv: -len(kv[1]))
    for (name, object_id), rows in npc[:6]:
        print(f"== {name} {object_id:016x} updates {len(rows)}")
        for row in rows[:8]:
            t, cell, x, y, z, motion, stamps = row
            print(f"  t={t:.3f} cell={cell:08x} pos=({x:.2f},{y:.2f},{z:.2f}) motion={motion:08x} stamps={stamps}")
        gaps = [b[0] - a[0] for a, b in zip(rows, rows[1:]) if b[0] > a[0]]
        if gaps:
            gaps.sort()
            print(f"  median interval {gaps[len(gaps)//2]:.3f}s")


if __name__ == "__main__":
    main()
