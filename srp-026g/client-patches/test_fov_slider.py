"""Field-of-view checks (patch 21 and the menu slider) that need neither the
game exe nor its archive."""
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

import fov_slider_026g as fov
import patch_026g as P


class FovTests(unittest.TestCase):
    def test_rewrites_keep_their_sites_size(self):
        for _, original, new in P.FOV_PATCHES:
            self.assertEqual(len(original), len(new))
        # The original projection is followed by four int3 bytes of padding.
        self.assertTrue(P.FOV_PROJECTION_ORIGINAL.endswith(b"\xcc" * 4))

    def test_replaced_addresses_are_where_the_disabled_relocations_are(self):
        self.assertEqual(P.FOV_PROJECTION_ORIGINAL[P.FOV_PROJECTION_RELOC - P.FOV_PROJECTION_VA:][:4],
                         struct.pack("<I", 0xA2D9EC))
        site, original, patched = P.FOV_MINIMUM
        self.assertEqual(original[P.FOV_MINIMUM_RELOC - site:][:4], struct.pack("<I", 0xAA6554))
        self.assertEqual(patched[2:10], struct.pack("<If", 0xAA6554, P.FOV_MIN))
        # The option's name is an address for an address: its relocation stays.
        site, original, patched = P.FOV_OPTION_NAME
        self.assertEqual((original[0], patched[0]), (0xB8, 0xB8))

    def test_projection_calls_the_original_matrix_builder(self):
        call = P.FOV_PROJECTION.index(b"\xe8")
        target = P.FOV_PROJECTION_VA + call + 5 + struct.unpack_from("<i", P.FOV_PROJECTION, call + 1)[0]
        original = P.FOV_PROJECTION_ORIGINAL.index(b"\xe8")
        self.assertEqual(target, P.FOV_PROJECTION_VA + original + 5
                         + struct.unpack_from("<i", P.FOV_PROJECTION_ORIGINAL, original + 1)[0])

    def test_projection_reads_uro_fov(self):
        self.assertIn(b"\xd8\x0d" + struct.pack("<I", P.FOV_VALUE), P.FOV_PROJECTION)

    def test_unknown_movie_is_rejected_before_invoking_compiler(self):
        with mock.patch.object(fov.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'original build 2777'):
                fov.build_movie(b'unknown movie', None, None, None, None)
            run.assert_not_called()

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'resources.db'
            output.write_bytes(b'existing archive')
            with mock.patch('sys.argv', ['fov_slider_026g.py', 'unused', str(output), '--ffdec=unused']):
                with self.assertRaises(SystemExit):
                    fov.main()
            self.assertEqual(output.read_bytes(), b'existing archive')


if __name__ == '__main__':
    unittest.main()
