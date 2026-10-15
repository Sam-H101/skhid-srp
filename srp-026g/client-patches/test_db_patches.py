"""db_patches.py: the same text as client/updater/logic/src/dbpatch.rs."""
import hashlib
import unittest

import db_patches as D


def sha(data):
    return hashlib.sha256(data).hexdigest()


# Revision 14: the stock db with the three live menus written in place, at
# their stock offsets (dbpatch.rs's tests have the same patch).
MENUS = [("write", 17970448, 1194596), ("write", 19165044, 38985769), ("write", 58150813, 3508744)]


def revision_14():
    return {"revision": 14, "from": (sha(b"stock"), 6285533906), "to": (sha(b"menus"), 6285533906),
            "blob": (sha(b"blob 14"), 9000000), "ops": list(MENUS)}


class FormatTests(unittest.TestCase):
    def test_a_patch_is_the_text_the_updater_reads(self):
        text = D.write_patch(revision_14())
        # Byte for byte the text of dbpatch.rs's a_patch_goes_to_text_and_back.
        self.assertEqual(text, (
            "skhid-db-patch 1\r\nrevision 14\r\n"
            f"from {sha(b'stock')} 6285533906\r\nto {sha(b'menus')} 6285533906\r\n"
            f"blob {sha(b'blob 14')} 9000000\r\n"
            "write 17970448 1194596\r\nwrite 19165044 38985769\r\nwrite 58150813 3508744\r\nend\r\n").encode())
        self.assertEqual(D.parse_patch(text), revision_14())
        self.assertEqual(D.parse_patch(text.replace(b"\r\n", b"\n")), revision_14())
        self.assertEqual(D.data_size(revision_14()), 1194596 + 38985769 + 3508744)
        for cut in range(len(text) - 4):
            with self.assertRaises((ValueError, IndexError)):
                D.parse_patch(text[:cut])

    def test_a_patch_breaking_a_rule_is_refused(self):
        grow = dict(revision_14(), to=(sha(b"items"), 6285533906 + 10), ops=MENUS + [("append", 10)])
        D.parse_patch(D.write_patch(grow))
        for ops, to in [
            (MENUS + [("append", 9)], grow["to"]),                       # the appends make `to`
            ([("write", 19165044, 10), ("write", 17970448, 10), ("append", 10)], grow["to"]),   # rising
            ([("write", 0, 100), ("write", 99, 1), ("append", 10)], grow["to"]),              # overlapping
            ([("write", 6285533906, 1), ("append", 10)], grow["to"]),    # inside the input
            ([], revision_14()["to"]),                                   # nothing done
            ([("append", 0)], revision_14()["to"]),
        ]:
            with self.subTest(ops=ops), self.assertRaises(ValueError):
                D.parse_patch(D.write_patch(dict(grow, ops=ops, to=to)))
        with self.assertRaises(ValueError):
            D.parse_patch(D.write_patch(revision_14()).replace(b"revision 14", b"revision 14\r\nrevision 15"))
        with self.assertRaises(ValueError):
            D.parse_patch(D.write_patch(revision_14()).replace(b"skhid-db-patch 1", b"skhid-db-patch 2"))

    def test_the_list_is_the_original_and_the_patches_in_order(self):
        first = revision_14()
        second = {"revision": 15, "from": first["to"], "to": (sha(b"items"), 6285533906 + 10),
                  "blob": (sha(b"blob 15"), 5), "ops": [("write", 0, 100), ("append", 10)]}
        original = (sha(b"stock"), 6285533906)
        text = D.write_list(original, [first, second])
        self.assertTrue(text.startswith(f"skhid-db-patches 1\r\noriginal {sha(b'stock')} 6285533906\r\n"
                                        "skhid-db-patch 1\r\nrevision 14\r\n".encode()))
        self.assertTrue(text.endswith(b"append 10\r\nend\r\nend\r\n"))
        self.assertEqual(D.parse_list(text), (original, [first, second]))
        self.assertEqual(D.write_list(original, []), (
            f"skhid-db-patches 1\r\noriginal {sha(b'stock')} 6285533906\r\nend\r\n").encode())
        for patches in ([second], [first, dict(second, revision=14)], [first, dict(second, **{"from": original})]):
            with self.subTest(patches=[p["revision"] for p in patches]), self.assertRaises(ValueError):
                D.write_list(original, patches)
        self.assertEqual(D.STOCK, ("c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c",
                                   6285533906))


if __name__ == "__main__":
    unittest.main()
