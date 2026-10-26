"""Write content/creature-entities.txt: every entity placed by the world content
files. import_creature_combat.py keeps only those whose portal.dat record has
health and natural damage, so NPCs without combat data never become creatures.

Usage: list_placed_entities.py
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "content"
SOURCES = ["world.tsv", "world-observed-expanded.tsv", "training-world.tsv", "capture-spawns.tsv"]


def main():
    found = set()
    for name in SOURCES:
        for line in (ROOT / name).read_text(encoding="utf-8").splitlines()[1:]:
            if line and not line.startswith("#"):
                found.add(int(line.split("\t")[1], 16))
    lines = ["pyrealium-creatures-v2",
             "# All placed entities; creature-combat.tsv keeps those with DAT combat data."]
    lines += [f"{entity:08x}" for entity in sorted(found)]
    (ROOT / "creature-entities.txt").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"placed entities {len(found)}")


if __name__ == "__main__":
    main()
