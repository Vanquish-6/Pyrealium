"""Read AC2 localized string tables (0x25xxxxxx in local_English.dat).

Layout and XoredReverse cipher follow the 2012 PortalExplorer
(FileTypes/StringTable.cs, Utility/Crypto/XoredReverse.cs). Substituted
strings keep their %XXXX% placeholders.

Usage: ac2_strings.py LOCAL_DAT PORTAL_DAT ENTITY_HEX [...]
"""
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ac2_dat import Dat

KEYS = [0xE5849238, 0xEB2D767D, 0xFC0FDD03, 0xCB45A6A6, 0x7F21E6BB, 0xF5A6CDC8, 0x22D9AD12, 0xB2A3D4DE,
        0x3B834DB7, 0xC406C91C, 0x8392305E, 0x0A9F0099, 0x0943B90C, 0xA66D953C, 0x0D929E1F, 0x0E2E9AD4]


def decrypt(data):
    out = bytearray(data)
    whole = len(data) - len(data) % 4
    for index, offset in enumerate(range(0, whole, 4)):
        value = struct.unpack_from("<I", data, offset)[0] ^ KEYS[index % 16]
        struct.pack_into("<I", out, offset, value)
    return bytes(((b >> 4) | (b << 4)) & 0xFF for b in out)


def parse_table(data):
    strings = {}
    if len(data) < 20:
        return strings
    pos = 16
    count = struct.unpack_from("<I", data, pos)[0]
    pos += 4
    for _ in range(count):
        if pos + 8 > len(data):
            break
        token, ref = struct.unpack_from("<II", data, pos)
        pos += 8
        if token == 0:
            break
        if ref != 0:
            continue
        sub, kind = struct.unpack_from("<HH", data, pos)
        pos += 4
        if kind == 0:
            sub -= 1
        if sub > 255:
            skip = struct.unpack_from("<H", data, pos)[0]
            pos += 2 + skip * 4 + 2
            continue
        parts, ids = [], []
        for part in range(sub):
            if not (part == 0 and kind == 1):
                pos += 4
            length = struct.unpack_from("<H", data, pos)[0]
            pos += 2
            raw = data[pos:pos + length * 2]
            pos += length * 2
            if (length * 2) % 4 == 0:
                pos += 2
            parts.append(decrypt(raw).decode("utf-16-le", "replace").strip() if length else "")
            if part == sub - 1:
                id_count = struct.unpack_from("<H", data, pos)[0]
                pos += 2
                ids = list(struct.unpack_from(f"<{id_count}I", data, pos))
                pos += id_count * 4 + 2
        text, d = "", 0
        if kind == 0 and ids:
            text, d = f"%{ids[0]:04X}%", 1
        for piece in parts:
            if d < len(ids):
                text += f"{piece}%{ids[d]:04X}%"
                d += 1
            else:
                text += piece
        strings.setdefault(token, text)
    return strings


class Strings:
    def __init__(self, local):
        self.dat, self.cache = local, {}

    def get(self, table, token):
        if table not in self.cache:
            try:
                self.cache[table] = parse_table(self.dat.read(table))
            except Exception:
                self.cache[table] = {}
        return self.cache[table].get(token)


def entity_name_ref(portal, entity):
    """(token, table) from the definition's gmWeenieDesc name, as examination.rs reads it."""
    did = 0x81000000 | (entity & 0xFFFFFF)
    data = portal.read(did)
    words = lambda offset: struct.unpack_from("<I", data, offset)[0]
    if words(0) & 1 == 0 or words(4) != did:
        return None
    desc, pos = words(8), 12
    if desc & 4 == 0:
        return None
    for bit in (1, 0x2000000, 2):
        if desc & bit:
            pos += 4
    token, table, extra = words(pos), words(pos + 4), words(pos + 8)
    if token == 0 or table >> 24 != 0x25 or extra != 0:
        return None
    return token, table


def main():
    with Dat(Path(sys.argv[1])) as local, Dat(Path(sys.argv[2])) as portal:
        local.index()
        portal.index()
        strings = Strings(local)
        for text in sys.argv[3:]:
            entity = int(text, 16)
            ref = entity_name_ref(portal, entity)
            print(f"{entity:08x}", ref and f"{ref[0]:08x}@{ref[1]:08x}", ref and strings.get(ref[1], ref[0]))


if __name__ == "__main__":
    main()
