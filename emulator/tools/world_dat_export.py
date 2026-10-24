"""Copy-on-export native AC2 DAT replacements and additions.

Layout evidence: recovered DiskFileInfo_t (84 bytes at file offset 300),
CLBlockAllocator::SaveFileInfo 0064e730 / Load_Data 0064e930,
BTree::LoadTree 0064cbb0 and SplitNodeExec 0064d030. Native block links reserve
their high bit for allocator state; exported offsets stay below 2 GiB.

Only a newly created private file is written. Original free-list links are kept
intact. Replaced records and the old tree remain reserved, unreachable blocks;
they are never incorrectly added to the free list. This conservative export
trades a little file space for preserving the source allocator's ownership.
"""
from bisect import bisect_left
import hashlib
import math
import os
from pathlib import Path
import shutil
import struct
import tempfile

from ac2_dat import Dat


HEADER_OFFSET = 300
HEADER_SIZE = 84
NODE_SIZE = 0x4cc
MAX_ENTRIES = 61
MIN_DEGREE = 31
MAX_RECORD_BYTES = 64*1024*1024
MAX_PATCH_BYTES = 256*1024*1024
MAX_RECORDS = 2_000_000
MAX_NATIVE_SIZE = 0x7fffffff
COPY_CHUNK = 4*1024*1024


def _fingerprint(path):
    stat = path.stat()
    return stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns


def _digest(path, limit=None, restored_header=None):
    digest = hashlib.sha256()
    remaining = limit
    with Path(path).open("rb") as file:
        while remaining is None or remaining > 0:
            data = file.read(COPY_CHUNK if remaining is None else min(COPY_CHUNK, remaining))
            if not data:
                if remaining:
                    raise ValueError("Archive was truncated during verification")
                break
            if restored_header is not None and file.tell() == len(data):
                data = restored_header + data[len(restored_header):]
            digest.update(data)
            if remaining is not None:
                remaining -= len(data)
    return digest.hexdigest()


def _copy(source, temporary):
    digest = hashlib.sha256()
    with source.open("rb") as reader, temporary.open("xb") as writer:
        while data := reader.read(COPY_CHUNK):
            digest.update(data)
            writer.write(data)
        writer.flush()
        os.fsync(writer.fileno())
    return digest.hexdigest()


class _Node:
    def __init__(self, entries=None, children=None):
        self.entries = entries or []
        self.children = children or []


def _split(parent, index):
    child = parent.children[index]
    median = child.entries[MIN_DEGREE-1]
    right = _Node(child.entries[MIN_DEGREE:], child.children[MIN_DEGREE:] if child.children else [])
    child.entries = child.entries[:MIN_DEGREE-1]
    if child.children:
        child.children = child.children[:MIN_DEGREE]
    parent.entries.insert(index, median)
    parent.children.insert(index+1, right)


def _insert_nonfull(node, entry):
    index = bisect_left([value[0] for value in node.entries], entry[0])
    if not node.children:
        node.entries.insert(index, entry)
        return
    if len(node.children[index].entries) == MAX_ENTRIES:
        _split(node, index)
        if entry[0] > node.entries[index][0]:
            index += 1
    _insert_nonfull(node.children[index], entry)


def _build_tree(entries):
    root = _Node()
    for entry in sorted(entries):
        if len(root.entries) == MAX_ENTRIES:
            root = _Node(children=[root])
            _split(root, 0)
        _insert_nonfull(root, entry)
    return root


def _append_chain(file, block_size, payload):
    start = file.seek(0, os.SEEK_END)
    if start % block_size:
        raise ValueError("Archive size is not aligned to its native block size")
    count = max(1, math.ceil(len(payload)/(block_size-4)))
    end = start+count*block_size
    if end > MAX_NATIVE_SIZE:
        raise ValueError("Export exceeds the native signed block-offset range")
    for index in range(count):
        block = bytearray(block_size)
        next_block = start+(index+1)*block_size if index+1 < count else 0
        struct.pack_into("<I", block, 0, next_block)
        data = payload[index*(block_size-4):(index+1)*(block_size-4)]
        block[4:4+len(data)] = data
        file.write(block)
    return start


def _write_tree(file, block_size, node):
    children = [_write_tree(file, block_size, child) for child in node.children]
    data = bytearray(NODE_SIZE)
    if children:
        struct.pack_into(f"<{len(children)}I", data, 0, *children)
    struct.pack_into("<I", data, 248, len(node.entries))
    for index, entry in enumerate(node.entries):
        struct.pack_into("<4I", data, 252+16*index, *entry)
    return _append_chain(file, block_size, data)


def inspect_tree(path):
    """Validate native key ranges, node occupancy, leaf depth and payload bounds."""
    with Dat(path) as archive:
        seen, entries, leaf_depths = set(), {}, set()

        def visit(offset, low, high, depth, root=False):
            if depth > 16 or offset in seen:
                raise ValueError("Cyclic or excessive native B-tree")
            seen.add(offset)
            data = archive.chain(offset, NODE_SIZE)
            count = struct.unpack_from("<I", data, 248)[0]
            if count > MAX_ENTRIES or (not root and count < MIN_DEGREE-1):
                raise ValueError("Invalid native B-tree node occupancy")
            keys = []
            for index in range(count):
                identity, start, size, stamp = struct.unpack_from("<4I", data, 252+16*index)
                if not low < identity < high or (keys and identity <= keys[-1]):
                    raise ValueError("Native B-tree keys violate their range")
                if identity in entries or size > MAX_RECORD_BYTES:
                    raise ValueError("Duplicate ID or excessive record size")
                if size and (start < archive.block_size or start % archive.block_size or start >= archive.size):
                    raise ValueError("Record starts outside its native archive")
                entries[identity] = (start, size, stamp)
                keys.append(identity)
                if len(entries) > MAX_RECORDS:
                    raise ValueError("Native B-tree exceeds the record limit")
            children = struct.unpack_from(f"<{count+1}I", data)
            if children[0]:
                if not count or not all(children):
                    raise ValueError("Incomplete native B-tree child array")
                boundaries = [low]+keys+[high]
                for index, child in enumerate(children):
                    visit(child, boundaries[index], boundaries[index+1], depth+1)
            else:
                if any(children):
                    raise ValueError("Invalid native leaf child array")
                leaf_depths.add(depth)

        visit(archive.root, -1, 0x100000000, 0, True)
        if len(leaf_depths) != 1:
            raise ValueError("Native B-tree leaves have unequal depths")
        return dict(records=entries, nodes=len(seen), depth=next(iter(leaf_depths)))


def export_archive(source, destination, patches, expected_hashes=None):
    """Export native payload replacements/additions to a *new* copied DAT.

    patches maps integer DIDs to bytes. expected_hashes optionally maps each
    edited DID to its original SHA256, or None to require a new, absent DID.
    Publication is no-replace and only occurs after complete index/payload checks.
    No existing destination, source archive, or hardlinked installation is written.
    """
    source, requested_destination = Path(source), Path(destination)
    if requested_destination.is_symlink():
        raise ValueError("Export destination must not be a symlink")
    source, destination = source.resolve(strict=True), requested_destination.resolve()
    if source == destination or destination.exists():
        raise FileExistsError("Choose a new DAT destination; existing files are never overwritten")
    if not destination.parent.is_dir():
        raise ValueError("Export destination parent must exist")
    if not isinstance(patches, dict) or len(patches) > 65536:
        raise ValueError("Export patch set exceeds its bounds")
    patches = dict(patches)
    total = 0
    for identity, data in patches.items():
        if type(identity) is not int or not 0 < identity <= 0xffffffff or not isinstance(data, bytes):
            raise ValueError("Export patches require u32 IDs and bytes payloads")
        if not 0 < len(data) <= MAX_RECORD_BYTES:
            raise ValueError("Edited record size exceeds export bounds")
        total += len(data)
    if total > MAX_PATCH_BYTES:
        raise ValueError("Export patch payloads exceed 256 MiB")
    before = _fingerprint(source)
    with source.open("rb") as file:
        original_header = file.read(0x180)
    if len(original_header) != 0x180:
        raise ValueError("DAT native header is truncated")
    header = original_header[HEADER_OFFSET:HEADER_OFFSET+HEADER_SIZE]
    magic, block_size, file_size = struct.unpack_from("<3I", header)
    if magic != 0x5442 or not 256 <= block_size <= 65536 or file_size != before[2] or file_size % block_size:
        raise ValueError("DAT native file-size/block header is invalid")
    if file_size > MAX_NATIVE_SIZE or any(original_header[0x100:0x12c]):
        raise ValueError("DAT has an active transaction or unsupported native size")
    if struct.unpack_from("<I", header, 44)[0]:
        raise ValueError("LRU-managed DAT exports are not supported")
    with Dat(source) as archive:
        originals = dict(archive.index())
        if len(originals) > MAX_RECORDS:
            raise ValueError("Source DAT exceeds export record bounds")
        if expected_hashes is not None:
            if not isinstance(expected_hashes, dict) or set(expected_hashes) != set(patches):
                raise ValueError("Expected hashes must describe every edited record")
            for identity, expected in expected_hashes.items():
                if expected is None:
                    if identity in originals:
                        raise ValueError(f"New record {identity:08x} already exists")
                else:
                    if identity not in originals or hashlib.sha256(archive.read(identity)).hexdigest() != expected:
                        raise ValueError(f"Source record {identity:08x} changed since this project was authored")
    patch_storage = sum(math.ceil(len(data)/(block_size-4))*block_size for data in patches.values())
    node_storage = math.ceil((len(originals)+len(patches)+1)/30)*math.ceil(NODE_SIZE/(block_size-4))*block_size
    required = file_size+patch_storage+node_storage+16*1024*1024
    if shutil.disk_usage(destination.parent).free < required:
        raise ValueError("Insufficient space for a complete isolated DAT export")
    stage = Path(tempfile.mkdtemp(prefix=".pyrealium-dat-export-", dir=destination.parent))
    temporary = stage/destination.name
    try:
        source_sha256 = _copy(source, temporary)
        if _fingerprint(source) != before:
            raise ValueError("Source archive changed while it was copied")
        updated = dict(originals)
        with temporary.open("r+b") as output:
            for identity, data in sorted(patches.items()):
                start = _append_chain(output, block_size, data)
                updated[identity] = (start, len(data), originals.get(identity, (0,0,0))[2])
            root = _write_tree(output, block_size, _build_tree([(identity, *entry) for identity, entry in updated.items()])) if patches else struct.unpack_from("<I", header, 32)[0]
            exported_size = output.seek(0, os.SEEK_END)
            expected_header = bytearray(original_header)
            struct.pack_into("<I", expected_header, HEADER_OFFSET+8, exported_size)
            struct.pack_into("<I", expected_header, HEADER_OFFSET+32, root)
            output.seek(0)
            output.write(expected_header)
            output.flush()
            os.fsync(output.fileno())
        with temporary.open("rb") as file:
            if file.read(0x180) != expected_header:
                raise ValueError("Export header verification failed")
        if _digest(temporary, file_size, original_header) != source_sha256:
            raise ValueError("Export changed original archive bytes outside its native header")
        tree = inspect_tree(temporary)
        if tree["records"] != updated:
            raise ValueError("Export B-tree inventory verification failed")
        with Dat(temporary) as verify:
            if verify.index() != updated:
                raise ValueError("Independent export inventory verification failed")
            for identity, data in patches.items():
                if verify.read(identity) != data:
                    raise ValueError(f"Export payload {identity:08x} did not round-trip")
        if _fingerprint(source) != before or _digest(source) != source_sha256:
            raise ValueError("Source archive changed during export")
        # A link from our freshly copied staging inode publishes without replacing
        # an existing destination on Windows or POSIX. The staging name is removed
        # immediately; final output is an independent inode with link count one.
        output_sha256 = _digest(temporary)
        os.link(temporary, destination)
        temporary.unlink()
        return dict(source=str(source), destination=str(destination), source_sha256=source_sha256,
                    output_sha256=output_sha256, original_bytes=file_size, output_bytes=exported_size,
                    records=len(updated), replacements=sum(identity in originals for identity in patches),
                    additions=sum(identity not in originals for identity in patches), nodes=tree["nodes"],
                    tree_depth=tree["depth"], source_unchanged=True,
                    allocation_policy="append-only; old payload/tree blocks remain reserved")
    finally:
        temporary.unlink(missing_ok=True)
        stage.rmdir()
