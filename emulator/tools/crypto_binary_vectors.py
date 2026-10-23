"""Read-only offline execution of recovered client's ISAAC instructions.

Requires pefile and unicorn. Does not start the client or access the network.
"""
import json
import argparse
import hashlib
import pathlib
import struct
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_ESP

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--client', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2] / 'ac2client.exe')
binary = parser.parse_args().client
if hashlib.sha256(binary.read_bytes()).hexdigest() != '9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796':
    raise ValueError('Unexpected client identity')
pe = pefile.PE(str(binary))
base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()

def words(*values):
    return struct.pack('<' + 'I' * len(values), *values)

def vector(seed):
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    uc.mem_map(base, (len(image) + 4095) & ~4095)
    uc.mem_write(base, image)
    uc.mem_map(0x10000000, 0x10000)
    obj, table, results, memory, stack, sentinel = [0x10000000 + n for n in (0, 0x100, 0x1000, 0x2000, 0x8000, 0xf000)]
    uc.mem_write(table, words(0, 0x65ff80, 0, 0x6603e0, 0x660280))
    uc.mem_write(obj, words(table, 0, results, memory, seed, seed, seed))
    uc.mem_write(stack, words(sentinel, obj + 4, 1))
    uc.reg_write(UC_X86_REG_ECX, obj)
    uc.reg_write(UC_X86_REG_ESP, stack)
    uc.emu_start(0x65ff80, sentinel, count=200000)
    batch = list(struct.unpack('<256I', uc.mem_read(results, 1024)))[::-1]
    # Cross a refill boundary using the original ISAAC instructions.
    uc.mem_write(stack, words(sentinel, obj + 4))
    uc.reg_write(UC_X86_REG_ECX, obj)
    uc.reg_write(UC_X86_REG_ESP, stack)
    uc.emu_start(0x6603e0, sentinel, count=200000)
    batch += list(struct.unpack('<256I', uc.mem_read(results, 1024)))[::-1]
    return {str(i): f'{batch[i]:08x}' for i in (0, 1, 2, 3, 254, 255, 256, 257, 511)}

print(json.dumps({str(s): vector(s) for s in (0, 1, 0x12345678, 0xffffffff)}, indent=2))
