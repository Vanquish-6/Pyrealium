"""Resolve a WSL function to supplied DAT bytecode, without executing it.

Layout: VTableSection::StreamPack 006e89d0 and
Resolver::GetFunctionOffset 00455460 in the matching build-7148 recovery.
Names and argument semantics are deliberately not inferred from numeric IDs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

from ac2_dat import Dat, wlib_sections


class Reader:
    def __init__(self, data):
        self.data = memoryview(data)
        self.position = 0

    def word(self):
        if self.position + 4 > len(self.data):
            raise ValueError('Truncated VTable word')
        value = struct.unpack_from('<I', self.data, self.position)[0]
        self.position += 4
        return value

    def array(self):
        count = self.word()
        if count > (len(self.data) - self.position) // 4:
            raise ValueError('VTable array exceeds section')
        values = struct.unpack_from(f'<{count}I', self.data, self.position)
        self.position += count * 4
        return values

    def nested(self):
        count = self.word()
        if count > (len(self.data) - self.position) // 4:
            raise ValueError('VTable outer array exceeds section')
        return tuple(self.array() for _ in range(count))


class Resolver:
    def __init__(self, sections):
        try:
            r = Reader(sections[6])
            self.mapper = r.nested()
            self.tables = r.nested()
            count = r.word()
            if count > (len(r.data) - r.position) // 8:
                raise ValueError('VTable package info exceeds section')
            r.position += count * 8  # size/checksum pairs
            self.packages = r.array()
            code = sections[1]
        except KeyError as error:
            raise ValueError('Missing WLib resolver/code section') from error
        if len(code) < 4 or struct.unpack_from('<I', code)[0] != len(code) - 4:
            raise ValueError('Invalid opcode stream length')
        self.code = code[4:]
        # The following package-name hash is not needed for numeric resolution.

    def offset(self, function, object_package=None):
        if not 0 <= function <= 0xffffffff:
            raise ValueError('Function ID must fit u32')
        package = (function >> 16) & 0x7fff
        if function & 0x80000000:
            object_package = package
        if object_package is None or not 0 <= object_package < len(self.packages):
            raise ValueError('Missing/out-of-range object package')
        try:
            slot = self.mapper[package][function & 0xffff]
            if slot == 0xffffffff:
                raise ValueError('Function has no local implementation')
            offset = self.tables[self.packages[object_package]][slot & 0xffff]
        except IndexError as error:
            raise ValueError('Function mapping is outside VTable') from error
        if offset % 4 or offset >= len(self.code):
            raise ValueError('Function offset is outside opcode stream')
        return offset

    def words(self, function, object_package=None, count=32):
        if not 1 <= count <= 256:
            raise ValueError('Word count must be 1..256')
        offset = self.offset(function, object_package)
        end = min(offset + count * 4, len(self.code))
        size = (end - offset) // 4
        return offset, struct.unpack_from(f'<{size}I', self.code, offset)

    def instructions(self, function, object_package=None, count=32):
        """Bounded instruction boundaries from OpcodeStream::WalkStream 006ec490.

        This is a linear window, not a control-flow graph or function extent.
        In particular, an immediate that resembles a call is never decoded as one.
        """
        if not 1 <= count <= 256:
            raise ValueError('Instruction count must be 1..256')
        offset = self.offset(function, object_package)
        result = []
        extended = {2, 6, 7, 8, 11, 12, 79, 80, 81, 82, 83, 84, 86}
        for _ in range(count):
            if offset + 4 > len(self.code):
                raise ValueError('Truncated instruction')
            word = struct.unpack_from('<I', self.code, offset)[0]
            opcode = word & 0xff
            size = 8 if opcode in extended else 4
            if opcode == 0x82:
                size = 12
            elif opcode == 0x2b:
                if offset + 12 > len(self.code):
                    raise ValueError('Truncated switch instruction')
                cases = struct.unpack_from('<I', self.code, offset + 8)[0]
                size = 12 + cases * 8
            if offset + size > len(self.code):
                raise ValueError('Instruction operands exceed opcode stream')
            operands = struct.unpack_from(f'<{size // 4 - 1}I', self.code, offset + 4)
            result.append({'offset': f'0x{offset:x}', 'word': f'0x{word:08x}',
                           'opcode': f'0x{opcode:02x}',
                           'operands': [f'0x{value:08x}' for value in operands]})
            offset += size
        return result


def number(text):
    return int(text, 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--portal', type=Path, required=True)
    parser.add_argument('--record', type=number, default=0x56000005)
    parser.add_argument('--function', type=number, required=True)
    parser.add_argument('--object-package', type=number)
    parser.add_argument('--words', type=int, default=32)
    parser.add_argument('--instructions', type=int,
                        help='Decode this many linear instruction boundaries (1..256)')
    args = parser.parse_args()
    with Dat(args.portal) as archive:
        record = archive.read(args.record)
    resolver = Resolver(wlib_sections(record))
    offset, words = resolver.words(args.function, args.object_package, args.words)
    output = {
        'record': f'0x{args.record:08x}',
        'record_sha256': hashlib.sha256(record).hexdigest(),
        'function': f'0x{args.function:08x}',
        'object_package': args.object_package,
        'code_offset': f'0x{offset:x}',
        'words': [f'0x{word:08x}' for word in words],
        'scope': 'Static numeric resolution only; not execution or argument semantics',
    }
    if args.instructions is not None:
        output['instructions'] = resolver.instructions(
            args.function, args.object_package, args.instructions)
        output['instruction_scope'] = 'Linear window only; not a function extent or control flow'
    print(json.dumps(output, indent=2))


if __name__ == '__main__':
    main()
