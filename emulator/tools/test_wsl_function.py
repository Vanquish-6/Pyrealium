import struct
import unittest

from wsl_function import Resolver


def words(*values):
    return struct.pack(f'<{len(values)}I', *values)


class FunctionResolutionTests(unittest.TestCase):
    def sections(self):
        return {
            # Two mapper packages, two concrete vtables, two size/checksum pairs,
            # and an object-package mapping that reverses concrete tables.
            6: words(2, 1, 0, 1, 1, 2, 2, 0, 4, 2, 8, 12,
                     2, 0, 0, 0, 0, 2, 1, 0),
            1: words(16, 10, 20, 30, 40),
        }

    def test_absolute_overrides_object_and_virtual_uses_object(self):
        r = Resolver(self.sections())
        self.assertEqual(r.words(0x80000000, 1, 2), (8, (30, 40)))
        self.assertEqual(r.words(0x00010000, 1, 2), (4, (20, 30)))
        self.assertEqual(r.offset(0x80010000), 4)
        with self.assertRaises(ValueError):
            r.offset(0)
        for function in (0x80020000, 0x80000001, -1, 0x100000000):
            with self.assertRaises(ValueError):
                r.offset(function)

    def test_truncated_tables_and_code_are_rejected(self):
        sections = self.sections()
        for size in range(len(sections[6])):
            with self.assertRaises(ValueError):
                Resolver({6: sections[6][:size], 1: sections[1]})
        for code in (b'', words(999, 1), words(0xffffffff)):
            with self.assertRaises(ValueError):
                Resolver({6: sections[6], 1: code})
        for offset in (1, 16, 0xffffffff):
            r = Resolver(sections)
            r.tables = ((offset, offset), (offset, offset))
            with self.assertRaises(ValueError):
                r.offset(0x80000000)
        r = Resolver(sections)
        r.mapper = ((0xffffffff,),)
        with self.assertRaises(ValueError):
            r.offset(0x80000000)

    def test_instruction_boundaries_skip_immediates_and_switch_cases(self):
        r = Resolver(self.sections())
        r.code = words(0x0b, 0x0c, 0x82, 1, 2, 0x2b, 99, 1, 0x0b, 0x0c, 0x0a)
        decoded = r.instructions(0, 1, 4)
        self.assertEqual([i['offset'] for i in decoded], ['0x0', '0x8', '0x14', '0x28'])
        self.assertEqual(decoded[0]['operands'], ['0x0000000c'])
        for code in (words(0x0b), words(0x82, 1), words(0x2b, 0),
                     words(0x2b, 0, 0xffffffff)):
            r.code = code
            with self.assertRaises(ValueError):
                r.instructions(0, 1, 1)
        with self.assertRaises(ValueError):
            r.instructions(0, 1, 257)


if __name__ == '__main__':
    unittest.main()
