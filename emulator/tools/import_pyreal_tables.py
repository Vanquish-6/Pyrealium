"""Import the recovered Pyreal insert dumps into server tables.

Portal rows are already the placed world (content/portals.json records the same
SHA256). This writes the player-level ratings and crafting curves.
"""
import hashlib
import re
from pathlib import Path

SOURCE = Path(r"C:\Users\chris\Downloads\3 emu2012 (Newest from Odn)\Documentation\Database")
ROOT = Path(__file__).resolve().parents[1]


def main():
    levels = (SOURCE / "Pyreal_DB_Insert_PlayerLevelData.sql").read_text(encoding="utf-8")
    craft = (SOURCE / "Pyreal_DB_Insert_CraftingData.sql").read_text(encoding="utf-8")
    level_sha = hashlib.sha256(levels.encode()).hexdigest()
    craft_sha = hashlib.sha256(craft.encode()).hexdigest()
    rows = re.findall(
        r"\((\d+),(\d+),(\d+),(\d+),(\d+),(\d+),(\d+),(\d+),(\d+),(\d+)\)",
        levels,
    )
    if len(rows) != 150 or rows[0][0] != "1" or rows[-1][0] != "150":
        raise SystemExit("player level dump is not the expected 1..150 table")
    out = [
        "//! Player level table from Pyreal_DB_Insert_PlayerLevelData.sql.",
        f"//! Source SHA256: {level_sha}",
        "//! Experience, health, vigor and skill points match the previous g1 table.",
        "//! Armor, shield, focus, hero credits and the XP cap are the remaining columns.",
        "use super::Level;",
        "pub const LEVELS: &[Level] = &[",
    ]
    for number, xp, health, vigor, credits, armor, shield, focus, heroes, cap in rows:
        out.append("    Level {")
        out.append(f"        number: {number},")
        out.append(f"        xp: {xp},")
        out.append(f"        health: {health},")
        out.append(f"        vigor: {vigor},")
        out.append(f"        credits: {credits},")
        out.append(f"        armor: {armor},")
        out.append(f"        shield: {shield},")
        out.append(f"        focus: {focus},")
        out.append(f"        hero_credits: {heroes},")
        out.append(f"        xp_cap: {cap},")
        out.append("    },")
    out.append("];")
    out.append("")
    (ROOT / "src/progression/levels.rs").write_text("\n".join(out), encoding="utf-8", newline="\n")

    lines = [
        "pyrealium-crafting-levels-v1",
        f"# Source SHA256: {craft_sha}",
        "# skill, level, curve XP, learning limit, crafting XP gained",
    ]
    count = 0
    for match in re.finditer(r"\((\d+),(\d+),(\d+),(\d+),(\d+)\)", craft):
        level, skill, curve, limit, gained = match.groups()
        lines.append(f"{int(skill):x}\t{level}\t{curve}\t{limit}\t{gained}")
        count += 1
    if count != 1750:
        raise SystemExit(f"expected 1750 crafting rows, found {count}")
    (ROOT / "content/crafting-levels.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"levels {level_sha}")
    print(f"crafting {craft_sha} rows {count}")


if __name__ == "__main__":
    main()
