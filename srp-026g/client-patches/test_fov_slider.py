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

    def test_replaced_address_is_where_the_disabled_relocation_is(self):
        site, original, patched = P.FOV_MINIMUM
        self.assertEqual(original[P.FOV_MINIMUM_RELOC - site:][:4], struct.pack("<I", 0xAA6554))
        self.assertEqual(patched[2:10], struct.pack("<If", 0xAA6554, P.FOV_MIN))
        # The option's name is an address for an address: its relocation stays.
        site, original, patched = P.FOV_OPTION_NAME
        self.assertEqual((original[0], patched[0]), (0xB8, 0xB8))

    def test_projection_hook_jumps_from_its_first_instruction(self):
        self.assertIn(P.FOV_PROJECTION_HOOK, P.LATE_JOIN_HOOKS)
        va, original, offset, opcode = P.FOV_PROJECTION_HOOK
        # `fld dword [ecx+8Ch]`, six bytes: a jmp and a nop.
        self.assertEqual((va, original, opcode), (0x242900, bytes.fromhex("d9818c000000"), 0xE9))
        self.assertLess(offset, P.LATE_JOIN_ASM_SIZE)

    def test_projection_code_reads_uro_fov_and_calls_the_matrix_builder(self):
        code = (Path(__file__).with_name("late_join_026g.asm")).read_text()
        body = code[code.index("fov_projection:"):code.index("fov_scope:")]
        self.assertIn(f"fld dword [{P.FOV_VALUE:#X}]".replace("0X", "0x"), body)
        self.assertIn("call 0xEEE30", body)
        self.assertTrue(body.rstrip().endswith("ret 8"))

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
