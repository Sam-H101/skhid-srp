"""Checks of the class update builder without requiring the proprietary game assets."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import class_update_026g as update


class ClassUpdateTests(unittest.TestCase):
    def test_unknown_movie_is_rejected_before_invoking_compiler(self):
        with mock.patch.object(update.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'Not a known live lobby movie'):
                update.build_movie(b'unknown movie', None, None, None, None)
            run.assert_not_called()

    def test_retail_movie_is_named_as_such(self):
        retail = hashlib.sha256(b'retail movie').hexdigest()
        with mock.patch.object(update, 'RETAIL_SHA256', retail), \
                mock.patch.object(update.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'retail lobby movie'):
                update.build_movie(b'retail movie', None, None, None, None, known=False)
            run.assert_not_called()

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'lobby_menu.swf'
            output.write_bytes(b'existing movie')
            with mock.patch('sys.argv', ['class_update_026g.py', 'unused', str(output), '--ffdec=unused']):
                with self.assertRaises(SystemExit):
                    update.main()
            self.assertEqual(output.read_bytes(), b'existing movie')

    def test_a_method_defined_twice_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'show'):
            update.check_source('function show():void {}\nfunction show():void {}', 'SrpRooms')
        update.check_source('function get x():int {}\nfunction set x(v:int):void {}', 'SrpRooms')

    def test_every_class_source_defines_each_method_once(self):
        for name, source in update.SOURCES.items():
            update.check_source(source.read_text(encoding='utf-8'), name)


if __name__ == '__main__':
    unittest.main()
