"""Native terrain payload, brush transaction and cross-block seam regressions."""
import math
import struct
import unittest

from world_terrain import TerrainEditor, decode_block, encode_block, decode_terrain_name
from preview_terrain import landscape_defs, decode_terrain


def u32(*values):
    return struct.pack("<"+"I"*len(values), *values)


def align(value):
    return value+bytes((-len(value))&3)


def native_string(value):
    # Test strings are short; this is the inverse native pack order.
    keys = (0xe5849238, 0xeb2d767d, 0xfc0fdd03)
    data = bytearray(((b>>4)|(b<<4))&255 for b in value.encode("ascii"))
    for offset in range(0, len(data)-3, 4):
        struct.pack_into("<I", data, offset, struct.unpack_from("<I",data,offset)[0]^keys[offset//4])
    return align(struct.pack("<H",len(data))+data)


def definitions():
    return struct.pack("<IfII3f6I256f", 0x36000000, 10., 2, 1, 1000., 0., 1.,
                       7,5,4,8,4,4, *(float(i) for i in range(256)))


def region():
    result = u32(0x0f000000,1)+native_string("Test")+struct.pack("<IfIIIII",0x41000000,4.,0x41000000,4,0x36000000,255,255)
    names = ("sound","scene","terrain","surface","encounters","detailmap","sky","water","fog","properties","worldmap")
    for name in names:
        identity = {"terrain":0x12000000,"surface":0x13000000}.get(name)
        result += u32(1,identity,0x80000001) if identity else u32(0)
    return result


def types():
    result = u32(0x12000000,0,0,2)
    for index, label in enumerate(("Dirt","GrassLand")):
        result += native_string(label)+struct.pack("<IIfII",0x80000002+index,index,.5,0,0)
    return result


def block(bx, by, value=100):
    did = bx<<24 | by<<16 | 65535
    result = u32(did,did-1,0,9)+bytes([value])*9+u32(9)+u32(*([0xabcde080]*9))
    result += b"\xfa"*((-len(result))&3)
    return result


class Archive:
    def __init__(self, records):
        self.records = records
    def index(self):
        return {did:(0,len(data),0) for did,data in self.records.items()}
    def read(self, did):
        return self.records[did]


class TerrainTests(unittest.TestCase):
    def setUp(self):
        self.records = {bx<<24 | by<<16 | 65535:block(bx,by) for bx in range(9,13) for by in range(9,13)}
        self.cells = Archive(self.records)
        self.portal = Archive({0x0f000000:region(),0x36000000:definitions(),0x12000000:types()})
        self.overlay = {}
        self.editor = TerrainEditor(self.cells,self.portal,self.overlay)
        self.cell = 0x0a0a0001

    def test_binary_roundtrip_preserves_references_packed_bits_and_padding(self):
        original = self.records[0x0a0affff]
        value = decode_block(original,self.editor.definitions)
        self.assertEqual(encode_block(value,self.editor.definitions),original)
        value["heights"][4] = 120
        encoded = encode_block(value,self.editor.definitions)
        self.assertEqual(encoded[:16],original[:16])
        self.assertEqual(encoded[25:],original[25:])
        self.assertEqual(decode_terrain(encoded,self.editor.definitions)["vertices"][4][2],120)

    def test_raise_updates_all_four_copies_of_corner_vertex(self):
        edits = self.editor.stroke(self.cell,[20,20,100],15,10,"raise")
        for did, index in ((0x0a0affff,8),(0x0b0affff,2),(0x0a0bffff,6),(0x0b0bffff,0)):
            self.assertIn(did,edits)
            before,after = edits[did]
            self.assertEqual(decode_block(after,self.editor.definitions)["heights"][index],110)
            self.assertEqual(self.cells.read(did),before)
        self.assertEqual(self.overlay,{})

    def test_paint_preserves_height_and_every_unrelated_packed_bit(self):
        edits = self.editor.stroke(self.cell,[20,20,100],15,1,"paint",terrain_type=1)
        self.assertEqual(len(edits),4)
        for before,after in edits.values():
            a,b = (decode_block(value,self.editor.definitions) for value in (before,after))
            self.assertEqual(a["heights"],b["heights"])
            for old,new in zip(a["infos"],b["infos"]):
                self.assertEqual(old&~127,new&~127)
                self.assertIn(new&127,(0,1))

    def test_flatten_and_lower_quantize_to_native_height_bytes(self):
        edits = self.editor.stroke(self.cell,[10,10,100],12,1,"flatten",target_height=80)
        center = decode_block(edits[0x0a0affff][1],self.editor.definitions)
        self.assertEqual(center["heights"][4],80)
        self.overlay.update({did:after for did,(before,after) in edits.items()})
        lowered = self.editor.stroke(self.cell,[10,10,80],12,5,"lower")
        self.assertEqual(decode_block(lowered[0x0a0affff][1],self.editor.definitions)["heights"][4],75)

    def test_smooth_reads_one_snapshot_instead_of_earlier_vertex_writes(self):
        source = decode_block(self.records[0x0a0affff],self.editor.definitions)
        source["heights"][4] = 180
        self.overlay[0x0a0affff] = encode_block(source,self.editor.definitions)
        edits = self.editor.stroke(self.cell,[10,10,180],15,1,"smooth")
        after = decode_block(edits[0x0a0affff][1],self.editor.definitions)
        self.assertEqual(after["heights"][4],109)  # (180+8*100)/9, nearest native entry.
        self.assertEqual(decode_block(self.overlay[0x0a0affff],self.editor.definitions)["heights"][4],180)

    def test_overlay_is_input_and_before_bytes_match_current_project(self):
        first = self.editor.stroke(self.cell,[10,10,100],12,5,"raise")
        self.overlay.update({did:after for did,(before,after) in first.items()})
        second = self.editor.stroke(self.cell,[10,10,105],12,5,"raise")
        self.assertEqual(second[0x0a0affff][0],first[0x0a0affff][1])
        self.assertEqual(decode_block(second[0x0a0affff][1],self.editor.definitions)["heights"][4],110)

    def test_nonmonotonic_height_table_is_not_bisected(self):
        self.editor.definitions["heights"][101] = 98
        self.editor.height_candidates = tuple(enumerate(self.editor.definitions["heights"]))
        edits = self.editor.stroke(self.cell,[10,10,100],11,1.9,"raise")
        self.assertEqual(decode_block(edits[0x0a0affff][1],self.editor.definitions)["heights"][4],102)

    def test_no_op_at_native_height_limit_does_not_make_project_records(self):
        self.records.update({did:block(did>>24,(did>>16)&255,255) for did in self.records})
        self.assertEqual(self.editor.stroke(self.cell,[10,10,255],12,10,"raise"),{})

    def test_invalid_inputs_and_unknown_terrain_are_transactional(self):
        original = dict(self.records)
        for options in (dict(radius=0),dict(radius=float("nan")),dict(strength=float("inf")),dict(mode="paint",terrain_type=99),dict(cell=0x0a0a0100)):
            args = dict(cell=self.cell,point=[10,10,100],radius=10,strength=1,mode="raise")
            args.update(options)
            with self.subTest(options=options),self.assertRaises(ValueError):
                self.editor.stroke(**args)
        self.assertEqual(self.records,original)
        self.assertEqual(self.overlay,{})

    def test_truncated_or_wrong_identity_records_are_refused(self):
        original = self.records[0x0a0affff]
        with self.assertRaises(ValueError):
            decode_block(original[:-1],self.editor.definitions)
        with self.assertRaises(ValueError):
            decode_block(original,self.editor.definitions,0x0b0bffff)

    def test_native_terrain_names_use_recovered_string_munging(self):
        self.assertEqual(decode_terrain_name(bytes.fromhex("7cc4e2f357c647")),"Default")
        self.assertEqual([value["label"] for value in self.editor.palette()],["Dirt","GrassLand"])


if __name__ == "__main__":
    unittest.main()
