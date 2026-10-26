"""Extract creature combat fields the 2012 World server reads from portal.dat.

Evidence: Entity.LoadFromPortalDat / ParseEntityChunk, Living.ParseEntityChunk,
Monster.ParseEntityChunk, EntityDataIndex (portal file 0x34000000), and
EmpPropNames in the 2012 Utility project. This does not execute the server
and does not invent values that are absent from the record or its parent.
"""
from __future__ import annotations

import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ac2_dat import Dat

MAX_HEALTH = 0x4100016E
NATURAL_DAMAGE = 0x41000184
NATURAL_VARIANCE = 0x41000185
NATURAL_VIGOR = 0x41000186
NATURAL_TIME = 0x41000187
ARMOR = 0x410000DE
DEATH_XP = 0x41000172
LEVEL = 0x41000173
HOMESICK = 0x41000178
PERCEPTION = 0x4100017A
DAMAGE_TYPE = 0x410002F9
# NPC_StartingSkill1..20 and NPC_StartingSkill1Level..20Level (EmpPropNames).
SKILL_TYPES = [0x410001BA, 0x410001BC, 0x410001BE, 0x410001C0, 0x410001C2,
               0x410001D3, 0x410001D5, 0x410001D7, 0x410001D9, 0x410001DB,
               0x410001E0, 0x410001E2, 0x410001E4, 0x410001E6, 0x410001E8,
               0x410001EA, 0x410001EC, 0x410001EE, 0x410001F0, 0x410001F2]
SKILL_LEVELS = list(range(0x410001F4, 0x41000208))
# SkillList: magic/melee/missile attack and defense.
ATTACK_SKILLS = {0x1, 0x7, 0xC}
DEFENSE_SKILLS = {0x2, 0x8, 0xD}

BOOL, INTEGER, FLOAT = 0x40000001, 0x40000002, 0x40000003
VECTOR, COLOR, STRING = 0x40000004, 0x40000005, 0x40000006
LABEL, FILEREF, UNKNOWN_INT = 0x40000007, 0x40000008, 0x40000009
STRTABLE, LONG, LOCATION = 0x4000000A, 0x4000000D, 0x4000000E
SIZES = {
    BOOL: 4, INTEGER: 4, FLOAT: 4, VECTOR: 12, COLOR: 4,
    LABEL: 4, FILEREF: 4, UNKNOWN_INT: 4, LONG: 8, LOCATION: 4,
}


class Reader:
    def __init__(self, data):
        self.data = data
        self.pos = 0

    def remaining(self):
        return len(self.data) - self.pos

    def u8(self):
        value = self.data[self.pos]
        self.pos += 1
        return value

    def u16(self):
        value = struct.unpack_from("<H", self.data, self.pos)[0]
        self.pos += 2
        return value

    def u32(self):
        value = struct.unpack_from("<I", self.data, self.pos)[0]
        self.pos += 4
        return value

    def i32(self):
        value = struct.unpack_from("<i", self.data, self.pos)[0]
        self.pos += 4
        return value

    def u64(self):
        value = struct.unpack_from("<Q", self.data, self.pos)[0]
        self.pos += 8
        return value

    def f32(self):
        value = struct.unpack_from("<f", self.data, self.pos)[0]
        self.pos += 4
        return value

    def skip(self, count):
        self.pos += count

    def skip_string(self):
        # EncryptedString with ASCII, the encoding Entity.ReadDefaultData uses.
        length = self.u16()
        padded = length + 2
        if padded % 4:
            padded += 4 - (padded % 4)
        self.skip(padded - 2)

    def skip_value(self, kind):
        if kind == STRING:
            self.skip_string()
        elif kind == STRTABLE:
            self.skip(12)
        elif kind in SIZES:
            self.skip(SIZES[kind])
        else:
            raise ValueError(f"unknown property type 0x{kind:08x}")


def load_index(archive):
    reader = Reader(archive.read(0x34000000))
    reader.skip(12)
    count = reader.u32()
    for _ in range(count):
        reader.u32()
        reader.skip_string()
    reader.skip(4)
    types = {}
    for _ in range(count):
        token = reader.u32()
        kind = reader.u32()
        reader.skip(16)
        types[token] = kind
    return types


def read_entity(archive, types, entity, stack=None):
    stack = stack or []
    if entity in stack or len(stack) > 8:
        return {}
    data = archive.read(entity)
    reader = Reader(data)
    if reader.remaining() < 12:
        return {}
    file_type = reader.u32()
    reader.u32()
    flags = reader.u32()
    values = {}
    if flags == 0x40000007:
        parent = reader.u32()
        values.update(read_entity(archive, types, parent, stack + [entity]))
    elif flags == 0x40000002:
        reader.u32()
    elif flags == 0x40000001:
        values["visual"] = reader.u32()
        if file_type == 0x2B:
            reader.skip(12)
    else:
        return values
    if file_type != 0x8B:
        return values
    chunks = reader.u32()
    for _ in range(chunks):
        reader.u32()
        count = reader.u32()
        for _ in range(count):
            token = reader.u32()
            kind = types.get(token)
            if kind is None:
                raise ValueError(f"0x{entity:08x} missing index type for 0x{token:08x}")
            if token == MAX_HEALTH:
                values["health"] = reader.i32()
            elif token == NATURAL_DAMAGE:
                values["damage"] = reader.u32()
            elif token == NATURAL_VARIANCE:
                values["variance"] = reader.f32()
            elif token == NATURAL_VIGOR:
                values["vigor"] = reader.u32()
            elif token == NATURAL_TIME:
                if kind == FLOAT:
                    values["time"] = reader.f32()
                else:
                    values["time"] = float(reader.u32())
            elif token == ARMOR:
                values["armor"] = reader.u32()
            elif token == DEATH_XP:
                values["xp"] = reader.u64()
            elif token == LEVEL:
                values["level"] = reader.u32()
            elif token in (HOMESICK, PERCEPTION) and kind == FLOAT:
                values["homesick" if token == HOMESICK else "perception"] = reader.f32()
            elif token == DAMAGE_TYPE:
                values["damage_type"] = reader.u32()
            elif token in SKILL_TYPES:
                values.setdefault("skills", {})[SKILL_TYPES.index(token)] = reader.u32()
            elif token in SKILL_LEVELS:
                values.setdefault("skill_levels", {})[SKILL_LEVELS.index(token)] = reader.u32()
            else:
                reader.skip_value(kind)
    return values


def interval_ms(time):
    if time is None or time <= 0:
        return 1500
    millis = int(round(time * 1000)) if time < 240 else int(round(time))
    return min(60000, max(250, millis))


def main():
    portal = Path(sys.argv[1])
    creatures = Path(sys.argv[2])
    output = Path(sys.argv[3])
    wanted = []
    for line in creatures.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or line.startswith("pyrealium-"):
            continue
        wanted.append(int(line, 16))
    with Dat(portal) as archive:
        archive.index()
        types = load_index(archive)
        rows = []
        failed = []
        for entity in wanted:
            try:
                values = read_entity(archive, types, entity)
            except Exception as error:
                failed.append((entity, str(error)))
                continue
            health = values.get("health") or 0
            damage = values.get("damage") or 0
            if health <= 0 or damage <= 0:
                continue
            skills = values.get("skills", {})
            levels = values.get("skill_levels", {})
            rating = lambda kinds: max((levels.get(slot, 0) for slot, skill in skills.items() if skill in kinds), default=0)
            variance = values.get("variance")
            if variance is None or not 0 <= variance <= 1:
                variance = 0.0
            radius = lambda key: round(values[key], 2) if values.get(key) and 0 < values[key] <= 500 else 0
            rows.append((
                entity, health, damage,
                interval_ms(values["time"]) if values.get("time") else 0,
                values.get("armor") or 0, values.get("xp") or 0, values.get("level") or 0,
                round(variance, 4), rating(ATTACK_SKILLS), rating(DEFENSE_SKILLS),
                radius("perception"), radius("homesick"), values.get("damage_type") or 0,
            ))
    lines = [
        "pyrealium-creature-combat-v2",
        "# Health, natural damage and variance, armor, death XP, level, attack/defense",
        "# skill levels, AI perception/homesick radii and damage type from portal.dat",
        "# entity records (inherited parents merged). interval_ms is natural weapon time",
        "# when present, otherwise 0. Radii of 0 mean the record had none.",
        "# Gold and attack range are not these fields.",
        "# entity health damage interval_ms armor xp level variance attack defense perception homesick damage_type",
    ]
    for row in rows:
        lines.append("\t".join(str(item) if index else f"{item:08x}" for index, item in enumerate(row)))
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"index {len(types)} creatures {len(wanted)} rows {len(rows)} failed {len(failed)}")
    if rows:
        healths = [row[1] for row in rows]
        damages = [row[2] for row in rows]
        print(f"health {min(healths)}..{max(healths)} damage {min(damages)}..{max(damages)}")
        print("sample", lines[4:9])
    for entity, error in failed[:8]:
        print(f"fail {entity:08x} {error}")


if __name__ == "__main__":
    main()
