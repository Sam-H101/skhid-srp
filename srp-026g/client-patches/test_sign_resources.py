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

    def run_main(self, *argv):
        """main() with these arguments: its exit code (0 when it returns)."""
        with mock.patch("sys.argv", ["sign_resources_026g.py", *argv]), \
                mock.patch("sys.stdout"), mock.patch("sys.stderr"):
            try:
                S.main()
            except SystemExit as stop:
                return stop.code
        return 0

    def test_the_key_is_given_with_or_without_an_equals_sign(self):
        for form in ([f"--key={self.key}"], ["--key", str(self.key)]):
            with self.subTest(form=form):
                data = bytearray(self.db.read_bytes())
                data[24 + 650:24 + 650 + S.SIGNATURE_SIZE] = bytes(S.SIGNATURE_SIZE)
                self.db.write_bytes(bytes(data))
                self.assertEqual(self.run_main("sign", str(self.db), *form), 0)
                self.assertEqual(self.which(), "skhid")

    def test_a_wrong_command_line_stops_it_and_signs_nothing(self):
        before = self.db.read_bytes()
        for argv in (["sign", str(self.db), "--key"], ["sign", str(self.db), "--kye", str(self.key)],
                     ["sign", str(self.db), str(self.key)], ["sign"], ["genkey", "--key=x"]):
            with self.subTest(argv=argv):
                self.assertEqual(self.run_main(*argv), S.USAGE_EXIT)
                self.assertEqual(self.db.read_bytes(), before)

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
