"""Tests of sign_resources_026g.py with a throwaway key (needs `openssl`)."""

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import sign_resources_026g as S

RETAIL_DB = os.environ.get("SKHID_RETAIL_DB", "/home/sheep/Projects/survarium/srp/0.26g/game/resources.db")


class SignTests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.key, public = Path(folder.name, "key.pem"), Path(folder.name, "key.der")
        S.genkey(str(self.key), str(public))
        patcher = mock.patch.object(S, "OWN_KEY", str(public))
        patcher.start()
        self.addCleanup(patcher.stop)
        table = b"a file table " * 50
        self.db = Path(folder.name, "resources.db")
        self.db.write_bytes(b"little-endian\0\0\0" + (1).to_bytes(4, "little") + len(table).to_bytes(4, "little")
                            + table + bytes(S.SIGNATURE_SIZE) + b"the first file")

    def which(self):
        with open(self.db, "rb") as handle:
            return S.which_key(*S.read_table(handle)[1:])

    def test_signed_with_the_key_it_is_skhid(self):
        self.assertIsNone(self.which(), "zeros sign nothing")
        S.sign_file(self.db, str(self.key))
        self.assertEqual(self.which(), "skhid")

    def test_a_changed_table_is_neither(self):
        S.sign_file(self.db, str(self.key))
        data = bytearray(self.db.read_bytes())
        data[30] ^= 1
        self.db.write_bytes(data)
        self.assertIsNone(self.which())
        with self.assertRaises(ValueError):
            S.sign_file(self.db, str(self.key))       # no room: neither zeros nor a signature

    @unittest.skipUnless(os.path.exists(RETAIL_DB), "the retail resources.db is not here")
    def test_the_retail_db_is_retail(self):
        with open(RETAIL_DB, "rb") as handle:
            self.assertEqual(S.which_key(*S.read_table(handle)[1:]), "retail")


if __name__ == "__main__":
    unittest.main()
