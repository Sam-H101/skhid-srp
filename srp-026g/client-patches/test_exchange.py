"""Checks of the exchange builder without requiring the proprietary game assets."""
import hashlib
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

import exchange_026g as exchange

HERE = Path(__file__).resolve().parent


class ExchangeTests(unittest.TestCase):
    def test_unknown_movie_is_rejected_before_invoking_compiler(self):
        with mock.patch.object(exchange.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'live build 2777'):
                exchange.build_movie(b'unknown movie', None, None, None, None)
            run.assert_not_called()

    def test_retail_movie_is_named_as_such(self):
        retail = hashlib.sha256(b'retail movie').hexdigest()
        with mock.patch.object(exchange, 'RETAIL_SHA256', retail), \
                mock.patch.object(exchange.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'retail lobby movie'):
                exchange.build_movie(b'retail movie', None, None, None, None)
            run.assert_not_called()

    def test_existing_output_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'lobby_menu.swf'
            output.write_bytes(b'existing entry')
            with mock.patch('sys.argv', ['exchange_026g.py', 'unused', str(output), '--ffdec=unused']):
                with self.assertRaises(SystemExit):
                    exchange.main()
            self.assertEqual(output.read_bytes(), b'existing entry')

    def run_main(self, source, output):
        built = b'N' * 16
        with mock.patch.object(exchange, 'MOVIE_SIZE', 16), \
                mock.patch.object(exchange, 'MOVIE_OFFSET', 8), \
                mock.patch.object(exchange, 'build_movie', return_value=built) as build, \
                mock.patch('sys.argv', ['exchange_026g.py', str(source), str(output), '--ffdec=unused']):
            exchange.main()
        return build.call_args.args[0]

    def test_an_entry_file_gives_an_entry_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, output = Path(temporary) / 'entry', Path(temporary) / 'out'
            source.write_bytes(b'E' * 16)
            self.assertEqual(self.run_main(source, output), b'E' * 16)
            self.assertEqual(output.read_bytes(), b'N' * 16)

    def test_an_archive_gives_an_archive_with_the_entry_replaced(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, output = Path(temporary) / 'resources.db', Path(temporary) / 'out'
            source.write_bytes(b'a' * 8 + b'E' * 16 + b'z' * 8)
            self.assertEqual(self.run_main(source, output), b'E' * 16)
            self.assertEqual(output.read_bytes(), b'a' * 8 + b'N' * 16 + b'z' * 8)

    def test_the_window_and_the_server_agree_on_the_buy_gold_mark(self):
        window = (HERE / 'exchange' / 'SrpExchange.as').read_text(encoding='utf-8')
        server = (HERE.parent / 'crates' / 'survarium' / 'src' / 'v026g' / 'lobby'
                  / 'requests.rs').read_text(encoding='utf-8')
        sent = re.search(r'BUY_GOLD:uint = (0x[0-9A-Fa-f]+);', window)
        read = re.search(r'pub const BUY_PREMIUM: u32 = (0x[0-9A-Fa-f_]+);', server)
        self.assertIsNotNone(sent)
        self.assertIsNotNone(read)
        self.assertEqual(int(sent.group(1), 16), int(read.group(1).replace('_', ''), 16))

    def test_every_language_has_every_text(self):
        window = (HERE / 'exchange' / 'SrpExchange.as').read_text(encoding='utf-8')
        texts = re.search(r'TEXT:Object = \{(.*?)\};', window, re.S).group(1)
        rows = re.findall(r'"(\w+)": \[(.*?)\]', texts, re.S)
        self.assertGreater(len(rows), 5)
        for key, row in rows:
            self.assertEqual(len(re.findall(r'"[^"]+"', row)), 5, key)


if __name__ == '__main__':
    unittest.main()
