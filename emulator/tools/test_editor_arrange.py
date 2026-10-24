"""Selection transforms preserve geometry, identities and transactional limits."""
import math
import unittest
from unittest.mock import MagicMock, patch

from editor_arrange import arrangement_rows, prepare_arrangement, remove_selection
from editor_document import rows_from_text


def row(identity, x, y=0, z=3, cell="02990109"):
    return [f"{identity:016x}", "47000001", "1f000001", cell, str(x), str(y), str(z), "1", "0", "0", "0"]


def manifest(rows):
    return "pyrealium-world-v2\n# Keep provenance\n" + "\n".join("\t".join(r) for r in rows) + "\n"


class ArrangementTests(unittest.TestCase):
    def setUp(self):
        self.rows = [row(0x8140000000020000, 0), row(0x8140000000020001, 2)]
        self.grid = dict(block_side=160, cell_side=20, cells=8)

    def test_rotate_about_centroid_preserves_spacing_tilt_scale(self):
        self.rows[0][7:11] = [str(math.sqrt(.5)), str(math.sqrt(.5)), "0", "0"]
        changed = arrangement_rows(self.rows, (5, 6, 7), degrees=90)
        self.assertEqual(changed[0][4:7], ["6", "7", "10"])
        self.assertEqual(changed[1][4:7], ["6", "5", "10"])
        self.assertEqual(changed[0][0], self.rows[0][0])
        q = [float(v) for v in changed[0][7:11]]
        self.assertAlmostEqual(sum(v*v for v in q), 1, places=6)
        self.assertNotEqual(q[1], 0)  # Tilt was composed, not flattened.

    def test_translation_preserves_exact_quaternion_text(self):
        self.rows[0][7:11] = ["0.707106781", "0", "0", "-0.707106781"]
        changed = arrangement_rows(self.rows, translation=(1, 2, 3))
        self.assertEqual(changed[0][7:11], self.rows[0][7:11])
        self.assertEqual(self.rows[0][4], "0")

    def test_outdoor_negative_and_cross_block_coordinates(self):
        rows = [row(1, 2, 5, cell="74ba0001"), row(2, 5, 5, cell="75ba0001")]
        changed = arrangement_rows(rows, (-10, 0, 0), landscape=self.grid)
        self.assertEqual(changed[0][3], "73ba0039")
        self.assertEqual(changed[0][4], "152")
        self.assertEqual(changed[1][3], "74ba0039")
        self.assertEqual(changed[1][4], "155")

    def test_spaced_array_uses_successive_offsets(self):
        changed = arrangement_rows(self.rows, (10, 0, 0), copies=3)
        self.assertEqual([float(r[4]) for r in changed], [10, 12, 20, 22, 30, 32])

    def test_grid_snap_uses_explicit_spacing(self):
        changed = arrangement_rows([row(1, .3, .7)], grid=.5)
        self.assertEqual(changed[0][4:7], ["0.5", "0.5", "3"])

    @patch("editor_arrange.Dat", MagicMock())
    def test_uniform_scale_upgrades_manifest_and_preserves_other_objects(self):
        original = manifest(self.rows)
        result, identities = prepare_arrangement(original, [int(self.rows[0][0], 16)], ".", scale=2)
        changed = rows_from_text(result)
        self.assertTrue(result.startswith("pyrealium-world-v3"))
        self.assertEqual(changed[0][-1], "2")
        self.assertEqual(changed[1], self.rows[1]+["-"])
        self.assertEqual(identities, [int(self.rows[0][0], 16)])

    def test_group_scaling_preserves_relative_spacing(self):
        changed = arrangement_rows(self.rows, scale=2)
        self.assertEqual([r[4] for r in changed], ["-1", "3"])
        self.assertEqual([r[-1] for r in changed], ["2", "2"])

    def test_mixed_room_selection_refused(self):
        rows = self.rows + [row(3, 3, cell="0299010a")]
        with self.assertRaisesRegex(ValueError, "same room"):
            arrangement_rows(rows)

    def test_invalid_and_excessive_transform_rejected(self):
        for options in (dict(copies=33), dict(degrees=float("nan")), dict(translation=(float("inf"), 0, 0)), dict(grid=-1)):
            with self.subTest(options=options), self.assertRaises(ValueError):
                arrangement_rows(self.rows, **options)

    @patch("editor_arrange.Dat", MagicMock())
    def test_duplicate_allocates_fresh_ids_without_changing_originals(self):
        original = manifest(self.rows)
        result, identities = prepare_arrangement(original, [int(r[0], 16) for r in self.rows], ".", translation=(5, 0, 0), copies=2)
        rows = rows_from_text(result)
        self.assertEqual(rows[:2], self.rows)
        self.assertEqual(len(rows), 6)
        self.assertEqual(len(set(identities)), 4)
        self.assertFalse(set(identities) & {int(r[0], 16) for r in self.rows})
        self.assertIn("# Keep provenance", result)

    @patch("editor_arrange.Dat", MagicMock())
    def test_per_cell_capacity_failure_is_atomic(self):
        original = manifest([row(0x8140000000020000+i, i) for i in range(64)])
        with self.assertRaisesRegex(ValueError, "limit"):
            prepare_arrangement(original, [0x8140000000020000], ".", copies=1)
        self.assertEqual(len(rows_from_text(original)), 64)

    def test_missing_dat_rejects_prepared_edit(self):
        with patch("editor_arrange.Dat", side_effect=ValueError("Missing DAT")):
            with self.assertRaisesRegex(ValueError, "Missing DAT"):
                prepare_arrangement(manifest(self.rows), [int(self.rows[0][0], 16)], ".", translation=(1, 0, 0))

    def test_delete_selection_preserves_comments_and_untouched_rows(self):
        result = remove_selection(manifest(self.rows), [int(self.rows[0][0], 16)])
        self.assertEqual(rows_from_text(result), [self.rows[1]])
        self.assertIn("# Keep provenance", result)
