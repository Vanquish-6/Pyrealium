"""Read-only AC2 DAT B-tree inventory and record extraction.

Layout evidence: recovered BTNode/BTEntry and BTree::LoadTree 0064cbb0.
Does not mutate archives, interpret record payloads, or apply other DAT overlays.
"""
from __future__ import annotations
import argparse
import json
import struct
from pathlib import Path


class Dat:
    def __init__(self, path):
        self.path = Path(path)
        self.file = self.path.open('rb')
        self.size = self.path.stat().st_size
        self.file.seek(0x12c)
        header = self.file.read(0x50)
        if len(header) != 0x50 or struct.unpack_from('<I', header)[0] != 0x5442:
            self.file.close()
            raise ValueError('Not a supported BT DAT archive')
        self.block_size = struct.unpack_from('<I', header, 4)[0]
        self.root = struct.unpack_from('<I', header, 0x20)[0]
        if self.block_size < 256 or self.block_size > 65536:
            raise ValueError('Invalid block size')
        self.entries = {}

    def close(self):
        self.file.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def chain(self, offset, length):
        if length < 0 or length > self.size:
            raise ValueError('Invalid record size')
        result = bytearray()
        seen = set()
        while len(result) < length:
            if offset in seen or offset < self.block_size or offset % self.block_size:
                raise ValueError(f'Invalid/cyclic block offset {offset:x}')
            if offset + self.block_size > self.size:
                raise ValueError('Block outside archive')
            seen.add(offset)
            self.file.seek(offset)
            block = self.file.read(self.block_size)
            offset = struct.unpack_from('<I', block)[0]
            result.extend(block[4:4 + min(length - len(result), self.block_size - 4)])
        return bytes(result)

    def index(self):
        if self.entries:
            return self.entries
        pending, seen = [self.root], set()
        while pending:
            offset = pending.pop()
            if offset in seen:
                raise ValueError('Cyclic B-tree')
            seen.add(offset)
            node = self.chain(offset, 0x4cc)
            count = struct.unpack_from('<I', node, 248)[0]
            if count > 61:
                raise ValueError('Invalid B-tree entry count')
            for i in range(count):
                did, start, length, stamp = struct.unpack_from('<4I', node, 252 + i * 16)
                if did in self.entries:
                    raise ValueError('Duplicate B-tree ID')
                self.entries[did] = (start, length, stamp)
            if struct.unpack_from('<I', node)[0]:
                pending.extend(struct.unpack_from(f'<{count + 1}I', node))
        return self.entries

    def read(self, did):
        if not self.entries:
            self.index()
        start, length, _ = self.entries[did]
        return self.chain(start, length)


def wlib_sections(data):
    """Return validated ByteStream sections from a WLib DAT record (version 15).

    Evidence: ByteStream::Pack/PackSection 006e4cd0/006e4d40. Values are raw
    section bytes; this deliberately does not claim to evaluate script logic.
    """
    if len(data) < 12 or data[4:8] != b'BS\x01\x1a':
        raise ValueError('Unsupported WLib magic')
    if struct.unpack_from('<I', data, 8)[0] != 15:
        raise ValueError('Unsupported ByteStream version')
    result, position = {}, 12
    while position < len(data):
        if position + 8 > len(data):
            raise ValueError('Truncated ByteStream section header')
        kind, size = struct.unpack_from('<II', data, position)
        end = position + 8 + size
        if end > len(data) or kind in result:
            raise ValueError('Invalid or duplicate ByteStream section')
        result[kind] = data[position + 8:end]
        position = (end + 3) & ~3
    if position != len(data):
        raise ValueError('Invalid ByteStream padding')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive')
    parser.add_argument('--id', type=lambda value: int(value, 0))
    parser.add_argument('--output', type=Path)
    parser.add_argument('--wlib-sections', action='store_true')
    args = parser.parse_args()
    with Dat(args.archive) as archive:
        archive.index()
        if args.id is None:
            print(json.dumps({f'{did:08x}': {'offset': item[0], 'size': item[1], 'stamp': item[2]}
                              for did, item in sorted(archive.entries.items())}, indent=2))
        else:
            data = archive.read(args.id)
            if args.output:
                with args.output.open('xb') as output:
                    output.write(data)
            print(json.dumps({'id': f'{args.id:08x}', 'size': len(data), 'prefix': data[:64].hex()}))
            if args.wlib_sections:
                print(json.dumps({hex(kind): len(section) for kind, section in wlib_sections(data).items()}))


if __name__ == '__main__':
    main()
