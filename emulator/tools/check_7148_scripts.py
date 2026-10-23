"""Read-only check for a known 7148 examination-script incompatibility.

This is a targeted diagnostic, not certification of all client/DAT compatibility.
No game code, game data, or executable modifications are distributed here.
"""
import argparse
import json
import struct
from pathlib import Path

from ac2_dat import Dat, wlib_sections

EXAMINATION_CALL = 0x009A0013
EXTENDED = {2, 6, 7, 8, 0xB, 0xC, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x56}


def examination_calls(section):
    """Walk serialized OpcodeStream instruction boundaries, not byte matches.

    Build 7148 OpcodeStream::WalkStream uses fixed-width words except its
    switch table. The leading word is the byte length of the instruction stream.
    """
    if len(section) < 4:
        raise ValueError('Missing opcode stream size')
    size = struct.unpack_from('<I', section)[0]
    code = memoryview(section)[4:]
    if size != len(code) or size % 4:
        raise ValueError('Invalid opcode stream size')
    position, found = 0, []
    while position < size:
        opcode = struct.unpack_from('<I', code, position)[0] & 0xFF
        length = 8 if opcode in EXTENDED else 4
        if opcode == 0x82:
            length = 12
        elif opcode == 0x2B:
            if position + 12 > size:
                raise ValueError('Truncated switch instruction')
            length = 12 + 8 * struct.unpack_from('<I', code, position + 8)[0]
        if position + length > size:
            raise ValueError('Truncated instruction')
        if opcode == 0xC and struct.unpack_from('<I', code, position + 4)[0] == EXAMINATION_CALL:
            found.append(position)
        position += length
    return found


def check(path):
    with Dat(path) as archive:
        sections = wlib_sections(archive.read(0x56000005))
    offsets = examination_calls(sections[1])
    return {
        'check': '7148 examination native call',
        'record': '0x56000005',
        'function': f'0x{EXAMINATION_CALL:08x}',
        'incompatible': bool(offsets),
        'call_offsets': [f'0x{offset:x}' for offset in offsets],
        'limitation': 'Checks one known missing native function only; a clean result does not certify compatibility.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('portal_dat', type=Path)
    args = parser.parse_args()
    result = check(args.portal_dat)
    print(json.dumps(result, indent=2))
    return 2 if result['incompatible'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
