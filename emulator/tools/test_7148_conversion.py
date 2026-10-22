"""Opt-in checks against the user's local assets; no proprietary fixture shipped."""
import os
import struct
import unittest
from pathlib import Path
from ac2_dat import Dat
from make_7148_test_assets import convert

@unittest.skipUnless(os.environ.get('AC2_TEST_GAME_DATA'), 'requires local DATs')
class ConversionTests(unittest.TestCase):
    def test_conversion_preserves_other_script_fields_and_rejects_drift(self):
        with Dat(Path(os.environ['AC2_TEST_GAME_DATA'])/'portal.dat') as dat:
            for did in (0x7100110F,0x7000037F):
                original=dat.read(did)
                converted=convert(original)
                self.assertEqual(len(converted),1752)
                self.assertEqual(struct.unpack_from('<I',converted,4)[0],1740)
                self.assertEqual(struct.unpack_from('<Q',converted,0x310)[0],0x7EA)
                self.assertEqual(converted[8:0x310],original[8:0x310])
                self.assertEqual(converted[0x318:0x62C],original[0x318:0x62C])
                self.assertEqual(converted[0x62C:],original[0x648:])
                changed=bytearray(original)
                changed[100]^=1
                with self.assertRaises(ValueError):convert(changed)
                with self.assertRaises(ValueError):convert(converted)

if __name__=='__main__': unittest.main()
