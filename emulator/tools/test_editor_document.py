"""Editor persistence regressions; DAT access is mocked, never native acceptance."""
import math
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch, MagicMock

from editor_document import (commit_document, prepare_change, read_document,
                             rows_from_text, heading_from_row)


class DocumentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.path = self.folder / "world.tsv"
        self.catalog = self.folder / "definitions.tsv"
        self.catalog.write_text("pyrealium-definitions-v1\n47000001\t1f000001\tTest creature\n", encoding="utf-8")
        self.row = "8140000000020000\t47000001\t1f000001\t02990109\t1\t2\t3\t0.707106781\t0\t0\t-0.707106781"
        self.original = "pyrealium-world-v2\n# Keep this comment\n" + self.row + "\n"
        self.path.write_text(self.original, encoding="utf-8")
        self.original = read_document(self.path)
        self.args = SimpleNamespace(catalog=self.catalog, game_data=self.folder, output=self.path,
                                    entity=0x47000001, cell=0x02990109, position=[4., 5., 6.], heading=80, scale=None)
        self.dat = patch("place_content.Dat", MagicMock())
        self.dat.start()
        self.addCleanup(self.dat.stop)

    def test_update_keeps_identity_comments_and_orientation(self):
        result, identity = prepare_change(self.args, self.original, int(self.row[:16], 16), preserve_orientation=True)
        row = rows_from_text(result)[0]
        self.assertEqual(identity, int(self.row[:16], 16))
        self.assertEqual(row[0], self.row[:16])
        self.assertEqual(row[4:7], ["4", "5", "6"])
        self.assertEqual(row[7:11], self.row.split("\t")[7:11])
        self.assertIn("# Keep this comment", result)
        self.assertEqual(read_document(self.path), self.original)

    def test_add_allocates_new_identity(self):
        result, identity = prepare_change(self.args, self.original)
        self.assertEqual(len(rows_from_text(result)), 2)
        self.assertNotEqual(identity, int(self.row[:16], 16))

    def test_scale_upgrade_preserves_other_rows(self):
        self.args.scale = 1.5
        result, _ = prepare_change(self.args, self.original)
        self.assertTrue(result.startswith("pyrealium-world-v3\n"))
        self.assertEqual(rows_from_text(result)[0], self.row.split("\t") + ["-"])
        self.assertEqual(rows_from_text(result)[1][-1], "1.5")

    def test_delete_backup_and_restore(self):
        result, _ = prepare_change(None, self.original, int(self.row[:16], 16), delete=True)
        commit_document(self.path, self.original, result)
        self.assertEqual(rows_from_text(read_document(self.path)), [])
        self.assertEqual(read_document(self.path.with_suffix(".tsv.bak")), self.original)
        commit_document(self.path, result, self.original)
        self.assertEqual(read_document(self.path), self.original)

    def test_external_change_is_not_overwritten(self):
        self.path.write_text(self.original + "# External change\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "outside the editor"):
            commit_document(self.path, self.original, self.original)
        self.assertTrue(read_document(self.path).splitlines()[-1] == "# External change")
        self.assertFalse(self.path.with_suffix(".tsv.edit.lock").exists())

    def test_lock_prevents_concurrent_save(self):
        self.path.with_suffix(".tsv.edit.lock").write_text("another editor", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            commit_document(self.path, self.original, self.original)
        self.assertEqual(read_document(self.path), self.original)

    def test_dat_failure_leaves_original_untouched(self):
        with patch("place_content.Dat", side_effect=ValueError("Missing DAT")):
            with self.assertRaisesRegex(ValueError, "Missing DAT"):
                prepare_change(self.args, self.original)
        self.assertEqual(read_document(self.path), self.original)

    def test_bad_orientation_rejected(self):
        with self.assertRaisesRegex(ValueError, "orientation"):
            rows_from_text(self.original.replace("0.707106781", "nan"))

    def test_heading_round_trip(self):
        for heading in (0, 32, 64, 128, 192, 254):
            q = heading * math.pi / 255
            row = self.row.split("\t")
            row[7:11] = [str(math.cos(q)), "0", "0", str(-math.sin(q))]
            self.assertEqual(heading_from_row(row), heading)


if __name__ == "__main__":
    unittest.main()
