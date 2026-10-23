"""Read movement-module state from one explicitly selected build-7148 process.

ReadProcessMemory only: no debugger attachment, code patches or input injection.
Addresses are from the matching supplied PDB/export, relocated to the live base.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
from pathlib import Path
import struct

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--pid', type=int, required=True)
parser.add_argument('--client', type=Path, default=Path(__file__).resolve().parents[2] / 'ac2client.exe')
args = parser.parse_args()
k = c.WinDLL('kernel32', use_last_error=True)
p = c.WinDLL('psapi', use_last_error=True)
k.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
k.OpenProcess.restype = w.HANDLE
k.CloseHandle.argtypes = [w.HANDLE]
k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
k.ReadProcessMemory.argtypes = [w.HANDLE, c.c_void_p, c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t)]
p.EnumProcessModulesEx.argtypes = [w.HANDLE, c.POINTER(w.HMODULE), w.DWORD, c.POINTER(w.DWORD), w.DWORD]
handle = k.OpenProcess(0x410, False, args.pid)
if not handle:
    raise c.WinError(c.get_last_error())
try:
    name = c.create_unicode_buffer(32768)
    length = w.DWORD(len(name))
    if not k.QueryFullProcessImageNameW(handle, 0, name, c.byref(length)):
        raise c.WinError(c.get_last_error())
    target = args.client
    actual = Path(name.value)
    if actual.resolve() != target.resolve():
        raise ValueError('Selected PID is not the supplied client executable')
    digest = hashlib.sha256(actual.read_bytes()).hexdigest()
    if digest != '9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796':
        raise ValueError('Unexpected client identity')
    modules = (w.HMODULE * 256)()
    needed = w.DWORD()
    if not p.EnumProcessModulesEx(handle, modules, c.sizeof(modules), c.byref(needed), 3):
        raise c.WinError(c.get_last_error())
    base = modules[0]

    def read(address, size):
        buf = c.create_string_buffer(size)
        done = c.c_size_t()
        if not k.ReadProcessMemory(handle, address, buf, size, c.byref(done)) or done.value != size:
            raise c.WinError(c.get_last_error())
        return buf.raw

    pointer = struct.unpack('<I', read(base + 0xb196cc - 0x400000, 4))[0]
    result = {'pid': args.pid, 'sha256': digest, 'image_base': hex(base), 'module': hex(pointer)}
    if pointer:
        state = read(pointer, 40)
        result.update(locked_actions=struct.unpack_from('<i', state)[0],
                      player_id=struct.unpack_from('<Q', state, 8)[0],
                      actively_moving=struct.unpack_from('<i', state, 16)[0],
                      last_position_send=struct.unpack_from('<d', state, 24)[0])
    print(json.dumps(result, indent=2))
finally:
    k.CloseHandle(handle)
