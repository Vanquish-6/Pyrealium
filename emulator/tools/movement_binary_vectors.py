"""Execute the supplied x86 Frame::cache_matrix as an offline heading oracle.

Requires pefile/unicorn, never launches or patches the native client.
"""
import hashlib
import argparse
import json
import math
from pathlib import Path
import struct
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_ESP

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--client', type=Path, default=Path(__file__).resolve().parents[2] / 'ac2client.exe')
binary = parser.parse_args().client
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
if digest != '9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796':
    raise ValueError('Unexpected client identity')
pe = pefile.PE(str(binary))
base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()


def vector(heading):
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    uc.mem_map(base, (len(image) + 4095) & ~4095)
    uc.mem_write(base, image)
    uc.mem_map(0x10000000, 0x10000)
    frame, stack, sentinel = 0x10000000, 0x10008000, 0x1000f000
    half = heading * math.pi / 255
    rotation = [math.cos(half), 0, 0, -math.sin(half)]
    uc.mem_write(frame + 12, struct.pack('<4f', *rotation))
    uc.mem_write(stack, struct.pack('<I', sentinel))
    uc.reg_write(UC_X86_REG_ECX, frame)
    uc.reg_write(UC_X86_REG_ESP, stack)
    uc.emu_start(0x619f70, sentinel, count=100000)
    axes = struct.unpack('<9f', uc.mem_read(frame + 28, 36))
    x, y = axes[3:5]
    angle = heading * 2 * math.pi / 255
    assert abs(x - math.sin(angle)) < 0.000001
    assert abs(y - math.cos(angle)) < 0.000001
    return {'heading': heading, 'native_forward': [x, y]}


print(json.dumps({'sha256': digest, 'function': 'Frame::cache_matrix 00619f70',
                  'vectors': [vector(h) for h in (0, 63, 64, 127, 128, 191, 192, 255)]}, indent=2))
