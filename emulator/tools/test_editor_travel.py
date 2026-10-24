"""Native atlas decoding and geographic click conversion regressions."""
import struct
import unittest

from editor_travel import map_surface, atlas_cell


class AtlasTests(unittest.TestCase):
    def test_native_bgr_map_pixels_decode_to_rgb(self):
        raw = struct.pack("<6I", 0x41000000, 0x8000000e, 2, 1, 20, 6) + bytes([10, 20, 30, 40, 50, 60])
        image = map_surface(raw, 0x41000000)
        self.assertEqual(image["ppm"], b"P6\n2 1\n255\n" + bytes([30, 20, 10, 60, 50, 40]))

    def test_truncated_or_misidentified_map_is_rejected(self):
        raw = struct.pack("<6I", 0x41000000, 0, 512, 512, 20, 512*512*3)
        with self.assertRaises(ValueError):
            map_surface(raw, 0x41000000)
        with self.assertRaises(ValueError):
            map_surface(raw, 0x41000001)

    def test_map_click_uses_native_landblock_and_cell_axes(self):
        # Point inside block (116,186), native cell X=2/Y=3.
        cell = atlas_cell((116+2.5/8)/255, 1-(186+3.5/8)/255, 255, 255, 8)
        self.assertEqual(cell, 0x74ba0014)

    def test_north_and_south_map_edges_stay_inside_native_bounds(self):
        self.assertEqual((atlas_cell(0, 0, 255, 255, 8) >> 16) & 255, 254)
        self.assertEqual((atlas_cell(0, .999999, 255, 255, 8) >> 16) & 255, 0)
        with self.assertRaises(ValueError):
            atlas_cell(1, .5, 255, 255, 8)
