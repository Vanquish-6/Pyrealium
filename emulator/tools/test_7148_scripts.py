import struct
import unittest

from check_7148_scripts import EXAMINATION_CALL, examination_calls


def stream(*words):
    return struct.pack('<I', 4 * len(words)) + struct.pack(f'<{len(words)}I', *words)


class ScriptCheckTests(unittest.TestCase):
    def test_actual_call_and_return_offset(self):
        self.assertEqual(examination_calls(stream(9, 0xC, EXAMINATION_CALL, 0x20005)), [4])

    def test_operands_and_switch_tables_are_not_instructions(self):
        self.assertEqual(examination_calls(stream(2, EXAMINATION_CALL, 0x2B, 0, 1, 0xC, EXAMINATION_CALL)), [])

    def test_truncated_and_unbounded_instructions(self):
        for data in [b'', stream(0xC), stream(0x82, 0), stream(0x2B, 0), stream(0x2B, 0, 0xFFFFFFFF), b'\x08\0\0\0\x09\0\0\0']:
            with self.assertRaises(ValueError):
                examination_calls(data)


if __name__ == '__main__':
    unittest.main()
