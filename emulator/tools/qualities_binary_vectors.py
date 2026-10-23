"""Run original x86 hash insertion to verify qualities table wire ordering.

Offline only; does not launch or modify the native client or DAT archives.
Requires pefile/unicorn, like the other original-binary verification tools.
"""
import hashlib
import argparse
import json
from pathlib import Path
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_ESP

from ac2_dat import Dat

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--client', type=Path, default=Path(__file__).resolve().parents[2] / 'ac2client.exe')
parser.add_argument('--game-data', type=Path, default=Path(__file__).resolve().parents[1] / 'target/compat-7148')
args = parser.parse_args()
binary = args.client
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
if digest != '9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796':
    raise ValueError('Unexpected client identity')
pe = pefile.PE(str(binary))
base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
pointer = struct.unpack('<I', pe.get_data(0xad703c - base, 4))[0]
bucket_sizes = struct.unpack('<23I', pe.get_data(pointer - base, 92))
assert bucket_sizes[0] == 11


def insert_native(entries):
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    uc.mem_map(base, (len(image) + 4095) & ~4095)
    uc.mem_write(base, image)
    uc.mem_map(0x10000000, 0x10000)
    table, buckets, nodes = 0x10000000, 0x10000100, 0x10001000
    stack, sentinel = 0x10008000, 0x1000f000
    uc.mem_write(table, struct.pack('<6I', 0, buckets, buckets + 44, 11, 0, 0))
    previous = 0
    for index, (key, value) in enumerate(entries):
        node = nodes + index * 12
        uc.mem_write(node, struct.pack('<3I', key, 0, value))
        uc.mem_write(stack, struct.pack('<3I', sentinel, node, previous))
        uc.reg_write(UC_X86_REG_ECX, table)
        uc.reg_write(UC_X86_REG_ESP, stack)
        uc.emu_start(0x004fc9f0, sentinel, count=10000)
        previous = node
    reachable = {}
    for bucket in range(11):
        node = struct.unpack('<I', uc.mem_read(buckets + bucket * 4, 4))[0]
        seen = set()
        while node:
            assert nodes <= node < nodes + len(entries) * 12 and node not in seen
            seen.add(node)
            key, following, value = struct.unpack('<3I', uc.mem_read(node, 12))
            assert key % 11 == bucket
            reachable[key] = value
            node = following
    count = struct.unpack('<I', uc.mem_read(table + 16, 4))[0]
    return count, reachable


with Dat(args.game_data / 'portal.dat') as dat:
    qualities = dat.read(0x8100110f)
assert len(qualities) == 204 and struct.unpack_from('<I', qualities, 52)[0] == 5
authored = [struct.unpack_from('<II', qualities, offset) for offset in range(56, 96, 8)]
assert insert_native(authored) == (5, dict(authored))
values = dict(authored)
values.update({key: 100 for key in (0x100, 0x101, 0x102, 0x103, 0x107, 0x108)})
values[0x12e] = 1
numeric = sorted(values.items())
correct = sorted(values.items(), key=lambda item: (item[0] % 11, item[0]))
bad_count, bad_reachable = insert_native(numeric)
count, reachable = insert_native(correct)
assert bad_count == 12 and len(bad_reachable) < bad_count
assert (count, reachable) == (12, values)
print(json.dumps({
    'sha256': digest,
    'function': 'add_for_unpacking 004fc9f0',
    'bucket_sizes': bucket_sizes,
    'numeric_order': {'stored_count': bad_count, 'reachable': len(bad_reachable)},
    'bucket_order': {'stored_count': count, 'reachable': len(reachable)},
    'wire_keys': [hex(key) for key, _ in correct],
}, indent=2))
