import hashlib
import struct
import unittest
from unittest.mock import patch

import repair_7148_examination as repair
from ac2_dat import wlib_sections
from check_7148_scripts import examination_calls


def fixture():
    code = bytearray(struct.pack('<I', 5) * ((repair.SITES[-1] + 16) // 4))
    for site in repair.SITES:
        code[site:site + 12] = repair.OLD
    section = struct.pack('<I', len(code)) + code
    return struct.pack('<I', repair.RECORD) + b'BS\x01\x1a' + struct.pack('<III', 15, 1, len(section)) + section


class ExaminationRepairTests(unittest.TestCase):
    def test_preserves_offsets_and_argument_cleanup(self):
        data = fixture()
        with patch.object(repair, 'SOURCE_HASH', hashlib.sha256(data).hexdigest()):
            result = repair.convert(data)
            self.assertEqual(len(result), len(data))
            self.assertEqual(examination_calls(wlib_sections(result)[1]), [])
            expected = bytearray(data)
            for site in repair.SITES:
                expected[24 + site:24 + site + 12] = repair.NEW
            self.assertEqual(result, expected)
            # The replacement's zero-count pops leave the original two-argument
            # cleanup intact, with no return value expected from the setter.
            self.assertEqual(sum(word >> 16 for word in struct.unpack('<III', repair.NEW)), 2)
            with self.assertRaises(ValueError):
                repair.convert(result)

    def test_rejects_unrecognized_data(self):
        with self.assertRaises(ValueError):
            repair.convert(fixture())

    def test_rejects_changed_argument_cleanup_even_with_matching_identity(self):
        data = bytearray(fixture())
        struct.pack_into('<I', data, 24 + repair.SITES[0] + 8, 0x10005)
        with patch.object(repair, 'SOURCE_HASH', hashlib.sha256(data).hexdigest()):
            with self.assertRaises(ValueError):
                repair.convert(data)


if __name__ == '__main__':
    unittest.main()
