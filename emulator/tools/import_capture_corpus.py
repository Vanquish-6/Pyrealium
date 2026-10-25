"""Parse retained packet captures for stationary spawns and keep new placements.

Existing lifestone, portal, observed, and training positions are left in place.
A capture that cannot be reassembled is reported and skipped.
"""
import json
from collections import Counter
from pathlib import Path

from import_observed_population import convert

ROOT = Path(__file__).resolve().parents[1]
GAME = Path(r"C:\Asheron's Call 2")
BASE = ROOT / "content" / "world.tsv"
OUTPUT = ROOT / "content" / "capture-spawns.tsv"
REPORT = ROOT / "content" / "capture-spawns-report.json"
ROOTS = [
    Path(r"C:\Users\chris\Downloads\ac2-dat-decompile\archives\retail2017"),
    Path(r"C:\Users\chris\Downloads\ac2-dat-decompile\archives\extra"),
    Path(r"C:\Users\chris\Downloads\3 emu2012 (Newest from Odn)\Packets"),
    Path(r"C:\Users\chris\Downloads\2 PyrealSourceCode\PyrealSourceCode\Users\User3\packets"),
    Path(r"C:\Users\chris\Downloads\2 PyrealSourceCode\PyrealSourceCode\Users\Ancaritha\DisassembledPackets"),
    Path(r"C:\Users\chris\Downloads\2 PyrealSourceCode\PyrealSourceCode\Users\User1\server_caps"),
]
PRIORITY = ("mobs", "world.objects", "dungeons", "npc", "shops", "combat", "loot")


def existing():
    positions, counts = set(), Counter()
    for path in (
        ROOT / "content" / "world.tsv",
        ROOT / "content" / "world-observed-expanded.tsv",
        ROOT / "content" / "training-world.tsv",
    ):
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line or line.startswith("#") or line.startswith("pyrealium-"):
                continue
            fields = line.split("\t")
            if len(fields) < 7:
                continue
            cell = int(fields[3], 16)
            counts[cell] += 1
            positions.add((fields[1], fields[3], *(round(float(v), 2) for v in fields[4:7])))
    return positions, counts


def captures():
    found = []
    for root in ROOTS:
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if path.suffix.lower() in {".elog", ".pcap", ".pcapng", ".trc"} and path.is_file():
                found.append(path)
    def rank(path):
        text = str(path).lower()
        for index, name in enumerate(PRIORITY):
            if name in text:
                return index
        return len(PRIORITY)
    found.sort(key=lambda path: (rank(path), str(path).lower()))
    unique, seen = [], set()
    for path in found:
        digest = path.stat().st_size, path.name.lower()
        if digest in seen:
            continue
        seen.add(digest)
        unique.append(path)
    return unique


def main():
    paths = captures()
    print(f"Reading {len(paths)} captures")
    _text, report = convert(paths, BASE, GAME, skip_broken=True)
    positions, counts = existing()
    fresh = []
    skipped = Counter()
    for actor in report["actors"]:
        key = (actor["entity"], actor["cell"], *(round(v, 2) for v in actor["position"]))
        cell = int(actor["cell"], 16)
        if key in positions:
            skipped["already-placed"] += 1
            continue
        if counts[cell] >= 64:
            skipped["cell-full"] += 1
            continue
        positions.add(key)
        counts[cell] += 1
        fresh.append(actor)
    lines = [
        "pyrealium-world-v3",
        "# Stationary objects parsed from retained captures and not already placed.",
        "# Not an authored spawn schedule. Modes, AI, loot and quest state are omitted.",
    ]
    for actor in sorted(fresh, key=lambda item: item["id"]):
        lines.append("\t".join([
            actor["id"], actor["entity"], actor["visual"], actor["cell"],
            *(format(v, ".9g") for v in actor["position"] + actor["orientation"]),
            "-" if actor["scale"] is None else format(actor["scale"], ".9g"),
        ]))
    OUTPUT.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    summary = {
        "captures": len(paths),
        "accepted_before_dedupe": report["placed_actors"],
        "added": len(fresh),
        "skipped": dict(skipped),
        "sources": report["sources"],
    }
    REPORT.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"Added {len(fresh)} new stationary objects from {report['placed_actors']} accepted observations.")
    print("skipped", dict(skipped))


if __name__ == "__main__":
    main()
