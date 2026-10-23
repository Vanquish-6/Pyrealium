"""Observe one explicitly selected local client; preserve first-chance fault context.

Waits for the user-launched executable, attaches using Windows debugging APIs,
then detaches after the first access violation. Does not alter the executable.
"""
import ctypes as c
from ctypes import wintypes as w
import json
import hashlib
import os
import sys
import time

k = c.WinDLL('kernel32', use_last_error=True)
HANDLE = w.HANDLE
k.CreateToolhelp32Snapshot.restype = HANDLE
k.OpenProcess.restype = HANDLE
k.OpenThread.restype = HANDLE
class Entry(c.Structure):
    _fields_ = [('size',w.DWORD),('usage',w.DWORD),('pid',w.DWORD),
                ('heap',c.c_size_t),('module',w.DWORD),('threads',w.DWORD),
                ('parent',w.DWORD),('priority',w.LONG),('flags',w.DWORD),
                ('name',w.WCHAR*260)]
class Event(c.Structure):
    _fields_ = [('code',w.DWORD),('pid',w.DWORD),('tid',w.DWORD),('data',c.c_uint64*20)]
k.Process32FirstW.argtypes = [HANDLE,c.POINTER(Entry)]
k.Process32NextW.argtypes = [HANDLE,c.POINTER(Entry)]
k.CloseHandle.argtypes = [HANDLE]
k.QueryFullProcessImageNameW.argtypes = [HANDLE,w.DWORD,w.LPWSTR,c.POINTER(w.DWORD)]
k.ReadProcessMemory.argtypes = [HANDLE,c.c_void_p,c.c_void_p,c.c_size_t,c.POINTER(c.c_size_t)]
k.Wow64GetThreadContext.argtypes = [HANDLE,c.c_void_p]
target = os.path.normcase(os.path.abspath(sys.argv[1]))
with open(target, 'rb') as original:
    if hashlib.file_digest(original, 'sha256').hexdigest() != '9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796':
        raise ValueError('Unexpected client identity; refusing debugger attachment')
deadline = time.monotonic()+1800
pid = None
while time.monotonic() < deadline and pid is None:
    snap = k.CreateToolhelp32Snapshot(2,0)
    entry = Entry(); entry.size=c.sizeof(entry)
    ok=k.Process32FirstW(snap,c.byref(entry))
    while ok:
        if entry.name.lower()==os.path.basename(target):
            hp=k.OpenProcess(0x1000,False,entry.pid)
            path=c.create_unicode_buffer(32768); size=w.DWORD(len(path))
            if hp and k.QueryFullProcessImageNameW(hp,0,path,c.byref(size)) and os.path.normcase(path.value)==target:
                pid=entry.pid
            if hp:k.CloseHandle(hp)
        ok=k.Process32NextW(snap,c.byref(entry))
    k.CloseHandle(snap)
    if pid is None:time.sleep(.1)
if pid is None:raise SystemExit('No matching user-launched client appeared')
if not k.DebugActiveProcess(pid):raise c.WinError(c.get_last_error())
k.DebugSetProcessKillOnExit(False)
hp=k.OpenProcess(0x410,False,pid)
trace_unpack = '--trace-unpack' in sys.argv[2:]
bp_address = 0x006dbd26
bp_original = None
if trace_unpack:
    # One-shot diagnostic breakpoint at HeapObject::UnPack's failure return.
    # Restore the original instruction before resuming; never save a patched EXE.
    k.CloseHandle(hp)
    hp=k.OpenProcess(0x438,False,pid)
    k.WriteProcessMemory.argtypes=[HANDLE,c.c_void_p,c.c_void_p,c.c_size_t,c.POINTER(c.c_size_t)]
    k.FlushInstructionCache.argtypes=[HANDLE,c.c_void_p,c.c_size_t]
    k.Wow64SetThreadContext.argtypes=[HANDLE,c.c_void_p]
    original=c.create_string_buffer(1); size=c.c_size_t()
    if not k.ReadProcessMemory(hp,bp_address,original,1,c.byref(size)) or original.raw != b'\x5f':
        k.DebugActiveProcessStop(pid)
        raise SystemExit('Unexpected 7148 instruction; refusing diagnostic breakpoint')
    if not k.WriteProcessMemory(hp,bp_address,c.c_char_p(b'\xcc'),1,c.byref(size)):
        k.DebugActiveProcessStop(pid)
        raise c.WinError(c.get_last_error())
    bp_original=original.raw
    k.FlushInstructionCache(hp,bp_address,1)
print(json.dumps({'attached_pid':pid,'executable':target}),flush=True)
def read_words(address, count):
    """Bounded, read-only snapshot; unreadable addresses return no words."""
    data = c.create_string_buffer(count * 4)
    size = c.c_size_t()
    k.ReadProcessMemory(hp, address, data, len(data), c.byref(size))
    return [int.from_bytes(data.raw[i:i+4], 'little')
            for i in range(0, size.value - size.value % 4, 4)]

def script_fault_context(stack):
    # Build 7148 GetPackageID and its specific GetSubDataIDs caller only.
    # At the fault, caller local_18 (ProcessState*) is at ESP+0x10.
    if len(stack) < 5 or stack[0] != 0x006b7c96:
        return {}
    process = read_words(stack[4], 13)
    result = {'process_address': hex(stack[4]),
              'process_words': [hex(x) for x in process]}
    if len(process) > 8:
        result['instance_reference'] = hex(process[8])
    # Capture the VM reference table metadata, not its potentially huge heap.
    globals_ = read_words(0x00baf440, 6)
    result['vm_globals'] = [hex(x) for x in globals_]
    if len(globals_) > 4:
        result['reference_table'] = [hex(x) for x in read_words(globals_[4], 8)]
    return result
try:
    while time.monotonic()<deadline:
        ev=Event()
        if not k.WaitForDebugEvent(c.byref(ev),500):continue
        status=0x10002
        stop=False
        if ev.code==1:
            raw=bytes(ev.data)
            code=int.from_bytes(raw[:4],'little')
            address=int.from_bytes(raw[16:24],'little')
            # WOW64 may report software breakpoints as STATUS_WX86_BREAKPOINT.
            is_breakpoint = code in (0x80000003, 0x4000001f)
            status=0x10002 if is_breakpoint else 0x80010001
            if trace_unpack:
                print(json.dumps({'debug_exception':hex(code),'address':hex(address)}),flush=True)
            if is_breakpoint and address==bp_address and bp_original:
                ctx=c.create_string_buffer(716)
                c.c_uint32.from_buffer(ctx).value=0x10007
                ht=k.OpenThread(0x58,False,ev.tid)
                if not k.Wow64GetThreadContext(ht,ctx):
                    raise c.WinError(c.get_last_error())
                values={name:c.c_uint32.from_buffer(ctx,off).value for name,off in [('ebx',164),('ebp',180),('esp',196)]}
                cursor=read_words(values['ebx'],1)
                print(json.dumps({'unpack_failure':hex(address),'heap_object':[hex(x) for x in read_words(values['ebp'],7)],'cursor_words':[hex(x) for x in read_words(cursor[0],24)] if cursor else [],'stack':[hex(x) for x in read_words(values['esp'],32)]}),flush=True)
                size=c.c_size_t()
                if not k.WriteProcessMemory(hp,bp_address,c.c_char_p(bp_original),1,c.byref(size)):
                    raise c.WinError(c.get_last_error())
                k.FlushInstructionCache(hp,bp_address,1)
                bp_original=None
                c.c_uint32.from_buffer(ctx,184).value=bp_address
                if not k.Wow64SetThreadContext(ht,ctx):
                    raise c.WinError(c.get_last_error())
                k.CloseHandle(ht)
            if code==0xc0000005:
                ctx=c.create_string_buffer(716)
                c.c_uint32.from_buffer(ctx).value=0x10007
                ht=k.OpenThread(0x48,False,ev.tid)
                good=k.Wow64GetThreadContext(ht,ctx)
                regs={name:hex(c.c_uint32.from_buffer(ctx,off).value) for name,off in [('edi',156),('esi',160),('ebx',164),('edx',168),('ecx',172),('eax',176),('ebp',180),('eip',184),('esp',196)]} if good else {}
                stack=c.create_string_buffer(512); read=c.c_size_t()
                if good:k.ReadProcessMemory(hp,int(regs['esp'],16),stack,len(stack),c.byref(read))
                words = [int.from_bytes(stack.raw[i:i+4],'little') for i in range(0,read.value,4)]
                script = script_fault_context(words) if address == 0x006dd7ae else {}
                print(json.dumps({'exception':hex(code),'address':hex(address),'first_chance':int.from_bytes(raw[152:156],'little'),'registers':regs,'stack_words':[hex(x) for x in words], 'script_state': script}),flush=True)
                k.CloseHandle(ht); stop=True
        k.ContinueDebugEvent(ev.pid,ev.tid,status)
        if stop or ev.code==5:break
finally:
    if bp_original:
        size=c.c_size_t()
        k.WriteProcessMemory(hp,bp_address,c.c_char_p(bp_original),1,c.byref(size))
        k.FlushInstructionCache(hp,bp_address,1)
    k.DebugActiveProcessStop(pid)
    k.CloseHandle(hp)
