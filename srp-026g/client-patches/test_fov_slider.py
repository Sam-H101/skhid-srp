"""FOV slider checks that need neither the game exe nor its archive."""
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

import fov_slider_026g as fov


class FovSliderTests(unittest.TestCase):
    def test_rewrites_keep_their_sites_size(self):
        for _, original, new in (fov.OPTION_NAME, fov.MINIMUM):
            self.assertEqual(len(original), len(new))
        self.assertEqual(len(fov.PROJECTION), len(fov.PROJECTION_ORIGINAL))
        # The original projection is followed by four int3 bytes of padding.
        self.assertTrue(fov.PROJECTION_ORIGINAL.endswith(b"\xcc" * 4))

    def test_absolute_addresses_sit_where_the_relocations_move(self):
        old, new = fov.PROJECTION_RELOC
        self.assertEqual(fov.PROJECTION_ORIGINAL[old - fov.PROJECTION_VA:][:4], struct.pack("<I", 0xA2D9EC))
        self.assertEqual(fov.PROJECTION[new - fov.PROJECTION_VA:][:4], struct.pack("<I", fov.URO_FOV))
        old, new = fov.MINIMUM_RELOC
        site, original, patched = fov.MINIMUM
        self.assertEqual(original[old - site:][:4], struct.pack("<I", 0xAA6554))
        self.assertEqual(patched[new - site:][:8], struct.pack("<If", 0xAA6554, fov.FOV_MIN))

    def test_projection_calls_the_original_matrix_builder(self):
        call = fov.PROJECTION.index(b"\xe8")
        target = fov.PROJECTION_VA + call + 5 + struct.unpack_from("<i", fov.PROJECTION, call + 1)[0]
        original = fov.PROJECTION_ORIGINAL.index(b"\xe8")
        self.assertEqual(target, fov.PROJECTION_VA + original + 5
                         + struct.unpack_from("<i", fov.PROJECTION_ORIGINAL, original + 1)[0])

    def test_unknown_movie_is_rejected_before_invoking_compiler(self):
        with mock.patch.object(fov.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'original build 2777'):
                fov.build_movie(b'unknown movie', None, None, None, None)
            run.assert_not_called()

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'resources.db'
            output.write_bytes(b'existing archive')
            for argv in (['resources', 'unused', str(output), '--ffdec=unused'], ['exe', 'unused', str(output)]):
                with mock.patch('sys.argv', ['fov_slider_026g.py'] + argv):
                    with self.assertRaises(SystemExit):
                        fov.main()
            self.assertEqual(output.read_bytes(), b'existing archive')


if __name__ == '__main__':
    unittest.main()
