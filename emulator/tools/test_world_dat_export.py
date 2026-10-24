"""Native DAT export tests with independent hand-built archive fixtures."""
import hashlib
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from ac2_dat import Dat
from world_dat_export import export_archive, inspect_tree


def fixture(path, count=40, block_size=256):
    """Build one native leaf and a marked free block without export helpers."""
    data = bytearray(max(512, block_size))
    entries = []
    payloads = {}
    for index in range(count):
        identity = 0x53000000+index
        payload = f"original-{index}".encode("ascii")
        offset = len(data)
        block = bytearray(block_size)
        block[4:4+len(payload)] = payload
        data.extend(block)
        entries.append((identity,offset,len(payload),1234+index))
        payloads[identity] = payload
    node = bytearray(0x4cc)
    struct.pack_into("<I",node,248,count)
    for index, entry in enumerate(entries):
        struct.pack_into("<4I",node,252+16*index,*entry)
    root = len(data)
    chunks = [node[index:index+block_size-4] for index in range(0,len(node),block_size-4)]
    for index, payload in enumerate(chunks):
        block = bytearray(block_size)
        struct.pack_into("<I",block,0,root+(index+1)*block_size if index+1<len(chunks) else 0)
        block[4:4+len(payload)] = payload
        data.extend(block)
    free = len(data)
    block = bytearray(block_size)
    struct.pack_into("<I",block,0,0x80000000)
    data.extend(block)
    prefix = b"Synthetic native BT DAT\0"
    data[:len(prefix)] = prefix
    struct.pack_into("<15I",data,300,0x5442,block_size,len(data),2,1,free,free,1,root,0,0,0,0,17,0)
    path.write_bytes(data)
    return payloads


class NativeExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.source = self.folder/"source.dat"
        self.destination = self.folder/"edited.dat"
        self.payloads = fixture(self.source)
        self.original = self.source.read_bytes()

    def test_replacement_addition_and_multiblock_payload_roundtrip(self):
        edits = {0x53000004:b"new replacement"*300,0x53000100:b"new room record"}
        expected = {0x53000004:hashlib.sha256(self.payloads[0x53000004]).hexdigest(),0x53000100:None}
        report = export_archive(self.source,self.destination,edits,expected)
        with Dat(self.destination) as dat:
            self.assertEqual(len(dat.index()),41)
            for identity, payload in self.payloads.items():
                self.assertEqual(dat.read(identity),edits.get(identity,payload))
            self.assertEqual(dat.read(0x53000100),edits[0x53000100])
        self.assertEqual(report["replacements"],1)
        self.assertEqual(report["additions"],1)
        self.assertEqual(self.source.read_bytes(),self.original)
        self.assertEqual(self.destination.stat().st_nlink,1)

    def test_native_multilevel_tree_ranges_and_leaf_depth(self):
        edits = {0x54000000+i:struct.pack("<I",i) for i in range(4000)}
        report = export_archive(self.source,self.destination,edits)
        self.assertGreaterEqual(report["tree_depth"],2)
        tree = inspect_tree(self.destination)
        self.assertEqual(len(tree["records"]),4040)
        with Dat(self.destination) as dat:
            self.assertEqual(dat.read(0x54000000+3999),struct.pack("<I",3999))
            self.assertEqual(dat.read(0x53000000),b"original-0")

    def test_allocator_free_chain_and_identity_header_are_preserved(self):
        export_archive(self.source,self.destination,{0x53000001:b"updated"})
        result = self.destination.read_bytes()
        for offset in range(384):
            if offset in range(308,312) or offset in range(332,336):
                continue
            self.assertEqual(result[offset],self.original[offset])
        first_free = struct.unpack_from("<I",result,320)[0]
        self.assertEqual(result[first_free:first_free+256],self.original[first_free:first_free+256])
        self.assertEqual(struct.unpack_from("<I",result,308)[0],len(result))

    def test_existing_or_hardlinked_destination_is_never_overwritten(self):
        os.link(self.source,self.destination)
        with self.assertRaises(FileExistsError):
            export_archive(self.source,self.destination,{0x53000000:b"bad"})
        self.assertEqual(self.source.read_bytes(),self.original)

    def test_same_source_destination_refused(self):
        with self.assertRaises(FileExistsError):
            export_archive(self.source,self.source,{0x53000000:b"bad"})
        self.assertEqual(self.source.read_bytes(),self.original)

    def test_stale_record_hash_refused_before_publish(self):
        with self.assertRaisesRegex(ValueError,"changed"):
            export_archive(self.source,self.destination,{0x53000000:b"edited"},{0x53000000:"0"*64})
        self.assertFalse(self.destination.exists())
        self.assertEqual(self.source.read_bytes(),self.original)

    def test_new_identity_must_be_absent_when_requested(self):
        with self.assertRaisesRegex(ValueError,"already exists"):
            export_archive(self.source,self.destination,{0x53000000:b"edited"},{0x53000000:None})

    def test_active_native_transaction_refused(self):
        data=bytearray(self.original)
        struct.pack_into("<I",data,256,1)
        self.source.write_bytes(data)
        with self.assertRaisesRegex(ValueError,"transaction"):
            export_archive(self.source,self.destination,{0x53000000:b"edited"})
        self.assertFalse(self.destination.exists())

    def test_publish_race_preserves_existing_file_and_cleans_stage(self):
        real_link = os.link
        def raced_link(source,destination):
            Path(destination).write_bytes(b"another export")
            return real_link(source,destination)
        with patch("world_dat_export.os.link",side_effect=raced_link):
            with self.assertRaises(FileExistsError):
                export_archive(self.source,self.destination,{0x53000000:b"edited"})
        self.assertEqual(self.destination.read_bytes(),b"another export")
        self.assertEqual(list(self.folder.glob(".pyrealium-dat-export-*")),[])

    def test_unsupported_lru_archive_refused(self):
        data=bytearray(self.original)
        struct.pack_into("<I",data,344,1)
        self.source.write_bytes(data)
        with self.assertRaisesRegex(ValueError,"LRU"):
            export_archive(self.source,self.destination,{0x53000000:b"edited"})

    @unittest.skipUnless(Path(r"C:\Asheron's Call 2\cell_1.dat").is_file(), "native cell fixture is not installed")
    def test_installed_native_archive_isolated_replacement_and_addition(self):
        source = Path(r"C:\Asheron's Call 2\cell_1.dat")
        with Dat(source) as archive:
            inventory = archive.index()
            terrain = next(identity for identity in sorted(inventory) if identity & 0xffff == 0xffff)
            original = archive.read(terrain)
            edited = bytearray(original)
            edited[16] ^= 1
            added = 0x0299f000
            while added in inventory:
                added += 1
        report = export_archive(source,self.destination,{terrain:bytes(edited),added:b"isolated export fixture"},
                                {terrain:hashlib.sha256(original).hexdigest(),added:None})
        self.assertTrue(report["source_unchanged"])
        self.assertEqual(report["records"],len(inventory)+1)
        self.assertEqual((report["replacements"],report["additions"]),(1,1))
        self.assertGreaterEqual(report["tree_depth"],2)

    def test_hardlinked_source_is_copied_without_changing_either_install_name(self):
        installation_alias = self.folder/"compat.dat"
        os.link(self.source,installation_alias)
        export_archive(installation_alias,self.destination,{0x53000000:b"new height data"})
        self.assertEqual(self.source.read_bytes(),self.original)
        self.assertEqual(installation_alias.read_bytes(),self.original)
        self.assertNotEqual(self.source.stat().st_ino,self.destination.stat().st_ino)
        self.assertEqual(self.source.stat().st_nlink,2)
        self.assertEqual(self.destination.stat().st_nlink,1)


if __name__ == "__main__":
    unittest.main()
