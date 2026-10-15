"""Check player-folder/ZIP contents without distributing or needing a game binary."""
import contextlib
import hashlib
import io
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
import unittest
from unittest import mock
import sys
import zipfile

import make_client_026g as overlay

S = overlay.S
# Throwaway keys standing for Skhid's ("skhid") and the original ("retail").
KEYS = tempfile.TemporaryDirectory(prefix='skhid-keys-')


def key_path(name, public=False):
    return str(Path(KEYS.name, name + ('.der' if public else '.pem')))


def setUpModule():
    for name in ('skhid', 'retail'):
        S.genkey(key_path(name), key_path(name, public=True))


def tearDownModule():
    KEYS.cleanup()

# A program that writes down how it was started (client/updater/tests).
STAND_IN = Path(overlay.HERE, '..', '..', 'client', 'updater', 'tests', 'fake_exe', 'target',
                'i686-pc-windows-msvc', 'release', 'fake-exe.exe').resolve()


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='skhid-release-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.game = self.root / 'install/game'
        self.exe = self.game / 'binaries/x86/survarium.exe'
        self.exe.parent.mkdir(parents=True)
        self.exe.write_bytes(b'original fixture')
        cert = self.game / overlay.CERT_REL
        cert.parent.mkdir(parents=True)
        cert.write_bytes(b'original certificate fixture')
        self.dll = self.root / 'bugtrap.dll'
        dll = bytearray(128)
        dll[:2] = b'MZ'
        dll[0x3c:0x40] = (64).to_bytes(4, 'little')
        dll[64:68] = b'PE\0\0'
        dll[68:70] = (0x14c).to_bytes(2, 'little')
        dll[86:88] = (0x2000).to_bytes(2, 'little')
        dll[88:90] = (0x10b).to_bytes(2, 'little')
        self.dll.write_bytes(dll)
        # The launcher's fixtures: an "original" launcher in the game folder
        # and an updater with the manifest the builder insists on.
        (self.exe.parent / 'survarium_launcher.exe').write_bytes(b'original launcher fixture')
        self.updater = self.root / 'updater_skhid.exe'
        updater = bytearray(dll)
        updater[86:88] = (0x0102).to_bytes(2, 'little')
        self.updater.write_bytes(updater + b"<requestedExecutionLevel level='asInvoker' uiAccess='false'/>")
        self.without_launcher = {'README.txt', 'LINUX.txt',
            'game/binaries/x86/survarium_skhid.exe', 'game/binaries/x86/bugtrap.dll',
            'game/binaries/x86/play_skhid.bat', 'game/binaries/x86/play_skhid.sh',
            'game/resources/ssl/survarium_login_server.crt'}
        self.names = self.without_launcher | {
            'game/binaries/x86/launcher_skhid.exe', 'game/binaries/x86/updater_skhid.exe',
            'game/binaries/x86/7zr.exe',
            'game/binaries/x86/updater_skhid.url', 'Skhid.bat', 'Skhid.sh',
        } | {f'game/binaries/x86/launcher_skhid/index-{blank}{code}.html'
             for blank in ('', 'blank-') for code in ('en', 'ru', 'ua', 'de', 'pl')} | {overlay.RESOURCES_PLAN}
        # The game's resources.db, as the default exe's release grows it.
        self.fake_db()

    def build(self, output, *options, plan=None, game_data_tool=None):
        """make_client_026g.py with these options. `srp-game-data plan` is
        stood in for: it writes `plan`'s blobs and gives its lines
        (`self.plan_calls` records how it was asked). The default exe's
        builds (no --resources-db, --no-late-join, --cert-only, --cave= or
        --no-launcher) get a release with a Skhid-signed table, which that
        build needs."""
        other_exe = ('--resources-db', '--no-late-join', '--cert-only', '--cave=', '--no-launcher')
        if plan is None and not any(option.startswith(other_exe) for option in options):
            plan = self.release_of()
            options += (f'--resources-db={self.root / "auto-final.db"}',
                        f'--resources-out={self.root / "auto-blobs"}')
        self.plan_calls = []

        def stand_in(tool, source, final_db, blobs):
            self.plan_calls.append((tool, source, final_db, blobs))
            lines, data = plan
            os.makedirs(blobs, exist_ok=True)
            for name, bytes_ in data.items():
                Path(blobs, name).write_bytes(bytes_)
            return lines
        args = ['make_client_026g.py', str(self.game), str(output),
                '--server=release.example:2234', '--localization=polish',
                '--bugtrap=' + str(self.dll), '--updater=' + str(self.updater), *options]
        # Binary patching has its own disassembly suite. Exercise every other
        # part of the real release builder, including source reads and ZIP I/O.
        # The server's game data is made by srp-game-data (its own tests):
        # here what the builder asks of it is recorded.
        launcher = mock.patch.object(
            overlay.L, 'build', return_value=(b'patched launcher fixture', 'test launcher'))
        self.game_data_calls = []
        tool = mock.patch.object(overlay, 'check_game_data_tool', side_effect=lambda path: path) \
            if game_data_tool is None else contextlib.nullcontext()
        made = mock.patch.object(overlay, 'server_game_data',
                                 side_effect=lambda *call: self.game_data_calls.append(call))
        planned = mock.patch.object(overlay, 'release_plan', side_effect=stand_in)
        keys = (mock.patch.object(S, 'OWN_KEY', key_path('skhid', public=True)),
                mock.patch.object(S, 'RETAIL_KEY', Path(key_path('retail', public=True)).read_bytes()))
        with mock.patch.object(overlay.P, 'build', return_value=(b'patched fixture', 'test patch')) as patcher:
            with launcher as self.launcher, mock.patch('sys.argv', args), tool, made, planned, keys[0], keys[1], \
                    contextlib.redirect_stdout(io.StringIO()) as messages:
                overlay.main()
        self.assertEqual(self.exe.read_bytes(), b'original fixture')
        return patcher.call_args, messages.getvalue()

    def test_folder_and_zip_ship_identical_player_files(self):
        folder, archive = self.root / 'overlay', self.root / 'release.zip'
        self.build(folder)
        self.build(archive, '--pack-zip')
        paths = {p.relative_to(folder).as_posix(): p for p in folder.rglob('*') if p.is_file()}
        self.assertEqual(set(paths), self.names)
        with zipfile.ZipFile(archive) as z:
            self.assertEqual(set(z.namelist()), self.names)
            self.assertIsNone(z.testzip())
            for name, path in paths.items():
                self.assertEqual(z.read(name), path.read_bytes(), name)
                info = z.getinfo(name)
                expected = 0o755 if name.endswith('.sh') else 0o644
                self.assertEqual(stat.S_IMODE(info.external_attr >> 16), expected)
                self.assertEqual(info.compress_type, zipfile.ZIP_DEFLATED)
        script = paths['game/binaries/x86/play_skhid.sh']
        if os.name != 'nt':
            self.assertEqual(stat.S_IMODE(script.stat().st_mode), 0o755)
        self.assertNotIn(b'\r', script.read_bytes())
        self.assertIn(b'localization=polish\nserver=\n', script.read_bytes())
        self.assertIn(b'LINUX.txt', paths['README.txt'].read_bytes())
        # The launcher is built from the game folder's own, for the server's
        # page, and starts the game with what play_skhid.bat starts it with.
        self.assertEqual(self.launcher.call_args.args[0], b'original launcher fixture')
        self.assertEqual(self.launcher.call_args.kwargs, dict(
            site='https://release.example', game_exe='survarium_skhid.exe',
            arguments='-mt_allocator=768 -minimum_resources_memory_size=768'))
        self.assertEqual(paths['game/binaries/x86/launcher_skhid.exe'].read_bytes(), b'patched launcher fixture')
        self.assertEqual(paths['game/binaries/x86/updater_skhid.exe'].read_bytes(), self.updater.read_bytes())
        self.assertEqual(paths['game/binaries/x86/updater_skhid.url'].read_bytes(), b'https://release.example\r\n')
        offline = paths['game/binaries/x86/launcher_skhid/index-ua.html'].read_text(encoding='utf-8')
        self.assertIn('href="https://release.example/"', offline)
        self.assertNotIn('survarium.com', offline)
        readme = paths['README.txt'].read_text(encoding='utf-8')
        # One file to start, beside `game`; nothing to type, and the archive
        # is downloaded once.
        self.assertIn('double-click Skhid.bat in that folder', readme)
        self.assertIn('You download this archive once', readme)
        self.assertNotIn('double-click game\\binaries', readme)
        self.assertIn('asks https://release.example whether', readme)
        starter = paths['Skhid.bat'].read_bytes()
        self.assertIn(b'start "" /d "%~dp0game\\binaries\\x86" "%~dp0game\\binaries\\x86\\launcher_skhid.exe"\r\n',
                      starter)
        self.assertEqual(starter.count(b'\n'), starter.count(b'\r\n'))
        # The same for Linux: a Bash script with Unix line ends.
        linux = paths['Skhid.sh'].read_bytes()
        self.assertTrue(linux.startswith(b'#!/usr/bin/env bash\n'))
        self.assertNotIn(b'\r', linux)
        self.assertIn(b'game/binaries/x86/play_skhid.sh', linux)
        self.assertIn('run bash Skhid.sh (the game) or bash Skhid.sh launcher', readme)

    @unittest.skipUnless(os.name == 'nt' and STAND_IN.is_file(),
                         'Windows, and the stand-in exe of client/updater/tests (build.sh there)')
    def test_the_starter_starts_the_launcher_from_its_folder(self):
        # A path with a space and a bracket, as a player's folder may have.
        install = self.root / 'My games (old)' / 'Survarium'
        folder = install / 'game' / 'binaries' / 'x86'
        folder.mkdir(parents=True)
        (install / 'Skhid.bat').write_bytes(overlay.starter())
        run = lambda: subprocess.run(  # noqa: E731
            ['cmd.exe', '/d', '/c', str(install / 'Skhid.bat')], cwd=self.root, stdin=subprocess.DEVNULL,
            capture_output=True, timeout=60)
        # Unpacked in the wrong place: it says where it belongs.
        missing = run()
        self.assertEqual(missing.returncode, 1)
        self.assertIn(b'launcher_skhid.exe was not found', missing.stdout)
        self.assertIn(b'the one that holds the', missing.stdout)
        # In its place: the launcher is started, in its own folder.
        launcher = folder / 'launcher_skhid.exe'
        shutil.copyfile(STAND_IN, launcher)
        started = run()
        self.assertEqual((started.returncode, started.stdout, started.stderr), (0, b'', b''))
        note = Path(str(launcher) + '.started.txt')
        for _ in range(100):
            if note.is_file() and note.stat().st_size:
                break
            time.sleep(0.1)
        command_line, working_folder = note.read_bytes().decode('utf-16').splitlines()[:2]
        self.assertEqual(command_line.strip().strip('"').lower(), str(launcher).lower())
        self.assertEqual(working_folder.lower(), str(folder).lower())

    def test_the_same_input_gives_the_same_archive(self):
        # The updater installs whenever the archive's checksum changes, so a
        # build must not differ from the one before by its own doing.
        first, second = self.root / 'first.zip', self.root / 'second.zip'
        self.build(first, '--pack-zip')
        self.build(second, '--pack-zip')
        self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_without_the_launcher_it_is_the_seven_files_of_before(self):
        archive = self.root / 'plain.zip'
        self.build(archive, '--pack-zip', '--no-launcher', '--no-late-join')
        self.launcher.assert_not_called()
        with zipfile.ZipFile(archive) as z:
            self.assertEqual(set(z.namelist()), self.without_launcher)
            readme = z.read('README.txt').decode('utf-8')
        self.assertIn('double-click game\\binaries\\x86\\play_skhid.bat', readme)
        self.assertNotIn('launcher_skhid', readme)

    def test_the_site_can_be_another_host_than_the_login_server(self):
        folder = self.root / 'other-site'
        self.build(folder, '--site=https://pages.example/srp/')
        self.assertEqual(self.launcher.call_args.kwargs['site'], 'https://pages.example/srp')
        self.assertEqual((folder / 'game/binaries/x86/updater_skhid.url').read_bytes(),
                         b'https://pages.example/srp\r\n')

    def test_an_updater_that_would_ask_for_administrator_rights_is_refused(self):
        self.updater.write_bytes(self.updater.read_bytes().replace(b'asInvoker', b'asWhoever'))
        with self.assertRaises(SystemExit) as refused:
            self.build(self.root / 'refused')
        self.assertIn('asInvoker', str(refused.exception))
        self.assertFalse((self.root / 'refused').exists())

    def test_the_extractor_shipped_is_the_one_the_updater_runs(self):
        # The updater runs 7zr.exe only when its size and sha256 are the two
        # compiled into it: the builder's numbers must be the updater's, and
        # the file in the archive the one with those numbers.
        import hashlib
        import re
        pins = Path(overlay.HERE, '..', '..', 'client', 'updater', 'logic', 'src', 'game.rs').read_text(encoding='utf-8')
        size, digest = re.search(
            r'pub const EXTRACTOR: Pin = Pin \{\s*size: ([0-9_]+),\s*sha256: "([0-9a-f]{64})",', pins).groups()
        self.assertEqual((int(size.replace('_', '')), digest), (overlay.EXTRACTOR_SIZE, overlay.EXTRACTOR_SHA256))
        self.assertIn(f'pub const EXTRACTOR_NAME: &str = "{overlay.EXTRACTOR_NAME}";', pins)
        archive = self.root / 'extractor.zip'
        self.build(archive, '--pack-zip')
        with zipfile.ZipFile(archive) as z:
            shipped = z.read('game/binaries/x86/7zr.exe')
            readme = z.read('README.txt').decode('utf-8')
        self.assertEqual((len(shipped), hashlib.sha256(shipped).hexdigest()),
                         (overlay.EXTRACTOR_SIZE, overlay.EXTRACTOR_SHA256))
        self.assertIn('7zr.exe is the standalone extractor of 7-Zip', readme)

    def test_another_extractor_than_the_pinned_one_is_refused(self):
        other = self.root / '7zr.exe'
        other.write_bytes(bytes(overlay.EXTRACTOR_SIZE))
        with self.assertRaises(SystemExit) as refused:
            self.build(self.root / 'refused', '--extractor=' + str(other))
        self.assertIn('not the extractor the updater is built to run', str(refused.exception))
        self.assertFalse((self.root / 'refused').exists())

    def test_certificate_only_launcher_has_an_explicit_server(self):
        archive = self.root / 'certificate.zip'
        call, _ = self.build(archive, '--pack-zip', '--cert-only')
        self.assertEqual(call.args[1:], ('cert-only', None, None, None))
        # The game's launcher passes the address too: this exe has none inside.
        self.assertEqual(self.launcher.call_args.kwargs['arguments'],
                         '-server=release.example:2234 -mt_allocator=768 -minimum_resources_memory_size=768')
        with zipfile.ZipFile(archive) as z:
            self.assertIn(b'server=release.example:2234\n', z.read('game/binaries/x86/play_skhid.sh'))

    def test_the_default_exe_has_the_late_join_and_can_be_built_without(self):
        code = Path(overlay.P.late_join_bin()).read_bytes()
        call, _ = self.build(self.root / 'with')
        self.assertEqual(call.args[1:], ('section', mock.ANY, 'release.example:2234', code))
        call, _ = self.build(self.root / 'without', '--no-late-join')
        self.assertEqual(call.args[1:], ('section', mock.ANY, 'release.example:2234', None))
        # The other cave has no room for it.
        call, _ = self.build(self.root / 'cave', '--cave=hash-mismatch')
        self.assertEqual(call.args[1:], ('hash-mismatch', mock.ANY, 'release.example:2234', None))

    def test_windows_checkout_produces_a_unix_launcher(self):
        # Git's core.autocrlf can give the release builder a CRLF template.
        template = Path(overlay.HERE, 'play_skhid.sh').read_bytes().replace(b'\r\n', b'\n')
        Path(self.root, 'play_skhid.sh').write_bytes(template.replace(b'\n', b'\r\n'))
        with mock.patch.object(overlay, 'HERE', str(self.root)):
            for embedded in (False, True):
                with self.subTest(embedded=embedded):
                    script = overlay.sh('release.example:2234', 'polish', embedded)
                    self.assertNotIn(b'\r', script)
                    server = b'' if embedded else b'release.example:2234'
                    self.assertIn(b'localization=polish\nserver=' + server + b'\n', script)

    def test_check_writes_neither_folder_nor_archive(self):
        for options in ((), ('--pack-zip',)):
            with self.subTest(options=options):
                output = self.root / 'not-created' / 'output'
                _, messages = self.build(output, *options, '--check')
                self.assertFalse(output.parent.exists())
                self.assertIn('play_skhid.sh', messages)
                self.assertIn('LINUX.txt', messages)

    # --- the players' resources.db ------------------------------------------

    def fake_db(self):
        """A small resources.db standing for the source game's."""
        db = os.urandom(4096)
        (self.game / 'resources.db').write_bytes(db)
        return db

    def release_of(self, head=True, regions=(('lobby_menu', 1000, 700),), tail=b'new entries ' * 100,
                   signer='skhid', claimed=None, required=True):
        """The plan `srp-game-data plan` gives for a release of the game's
        db, and its blobs: a new head (a header, a file table and its
        signature by `signer`'s key, or none; `claimed` the table size its
        header gives, when not the table's own) when `head`, the `regions`
        changed in place, and the `tail`."""
        db = (self.game / 'resources.db').read_bytes()
        sha = lambda data: hashlib.sha256(data).hexdigest()  # noqa: E731
        lines, blobs = [], {}

        def region(offset, before, after, name):
            lines.append(f'region game/resources.db {len(db)} {offset} {len(before)} {sha(before)} '
                         f'{sha(after)} {name}')
            blobs.update({sha(before): before, sha(after): after})
        if head:
            table = (b'new table ' + db)[:500 - 24 - S.SIGNATURE_SIZE - 20]
            header = b'little-endian\0\0\0' + (1).to_bytes(4, 'little') + len(table).to_bytes(4, 'little')
            path = self.root / 'head.bin'
            path.write_bytes(header + table + bytes(S.SIGNATURE_SIZE) + db[24 + len(table) + 256:500])
            if signer:
                S.sign_file(path, key_path(signer))
            new = bytearray(path.read_bytes())
            new[20:24] = (claimed or len(table)).to_bytes(4, 'little')
            region(0, db[:500], bytes(new), 'resources_head')
        for name, offset, size in regions:
            region(offset, db[offset:offset + size], b'changed ' + db[offset + 8:offset + size], name)
        if tail:
            lines.append(f'tail game/resources.db {len(db)} {len(db) + len(tail)} {sha(tail)} '
                         'resources_tail resources_head')
            blobs[sha(tail)] = tail
        elif head and required:
            lines.append('required game/resources.db')
        return lines, blobs

    def test_a_release_has_the_plan_and_the_blobs_of_its_resources_db(self):
        db = self.fake_db()
        lines, data = self.release_of(regions=(('login_menu', 2500, 300), ('lobby_menu', 1000, 700)))
        blobs, final_db = self.root / 'blobs', self.root / 'final.db'
        archive = self.root / 'release.zip'
        self.build(archive, '--pack-zip', f'--resources-db={final_db}', f'--resources-out={blobs}',
                   plan=(lines, data))
        # The tool compares the game's db with the final one, into the blobs' folder.
        self.assertEqual(self.plan_calls, [(overlay.GAME_DATA_TOOL, str(self.game), str(final_db), str(blobs))])
        with zipfile.ZipFile(archive) as z:
            self.assertEqual(set(z.namelist()), self.names)
            plan = z.read(overlay.RESOURCES_PLAN).decode()
        # Format 2 (a tail), the regions in the order of the db, then the tail.
        self.assertEqual(plan.split('\r\n'), ['skhid-resources 2', lines[0], lines[2], lines[1], lines[3], 'end', ''])
        self.assertEqual({p.name: p.read_bytes() for p in blobs.iterdir()}, data)
        self.assertEqual((self.game / 'resources.db').read_bytes(), db, 'the game is only read')
        # Without a tail the plan stays format 1 (the updaters out there).
        self.assertTrue(overlay.resources_plan(['region game/resources.db 1 0 1 a b x']).startswith(
            b'skhid-resources 1\r\n'))

    def test_a_release_of_menus_alone_keeps_the_original_table(self):
        self.fake_db()
        lines, data = self.release_of(head=False, tail=b'')
        archive = self.root / 'menus.zip'
        # The exe that starts with the original table: regions alone.
        self.build(archive, '--pack-zip', '--no-late-join', f'--resources-db={self.root / "final.db"}',
                   f'--resources-out={self.root / "blobs"}', plan=(lines, data))
        with zipfile.ZipFile(archive) as z:
            self.assertEqual(z.read(overlay.RESOURCES_PLAN).decode(),
                             f'skhid-resources 1\r\n{lines[0]}\r\nend\r\n')

    def test_a_new_table_without_a_tail_is_a_required_plan_of_format_3(self):
        """The menus with a table Skhid signed and no new files: the plan is
        required all the same (the game from revision 12 on does not start
        with the old table), so it is of format 3, which an updater of
        format 2 refuses whole."""
        self.fake_db()
        lines, data = self.release_of(tail=b'')
        archive = self.root / 'table.zip'
        self.build(archive, '--pack-zip', f'--resources-db={self.root / "final.db"}',
                   f'--resources-out={self.root / "blobs"}', plan=(lines, data))
        with zipfile.ZipFile(archive) as z:
            plan = z.read(overlay.RESOURCES_PLAN).decode()
        self.assertEqual(plan.split('\r\n'), ['skhid-resources 3', lines[0], lines[1],
                                               'required game/resources.db', 'end', ''])
        # A plan with a new table that is not required: refused.
        with self.assertRaises(SystemExit) as refused:
            self.build(self.root / 'refused.zip', '--pack-zip', f'--resources-db={self.root / "final.db"}',
                       f'--resources-out={self.root / "blobs2"}', plan=self.release_of(tail=b'', required=False))
        self.assertIn('not required', str(refused.exception))
        self.assertFalse((self.root / 'refused.zip').exists())

    def test_a_zip_that_changes_the_db_comes_with_the_servers_data(self):
        self.fake_db()
        out = self.root / 'release'
        archive = out / 'Survarium-0.26g-srp-files.zip'
        final_db, blobs = self.root / 'final.db', f'--resources-out={self.root / "blobs"}'
        release = (f'--resources-db={final_db}', blobs)
        self.build(archive, '--pack-zip', *release, plan=self.release_of())
        # Made from the final db and the ZIP, beside the ZIP.
        self.assertEqual(self.game_data_calls, [(
            overlay.GAME_DATA_TOOL, str(final_db), str(archive), str(out / overlay.SERVER_GAME_DATA))])
        # --check says so, makes nothing, and leaves no blobs.
        check_blobs = self.root / 'check-blobs'
        _, said = self.build(self.root / 'check' / 'x.zip', '--pack-zip', '--check', f'--resources-db={final_db}',
                             f'--resources-out={check_blobs}', plan=self.release_of())
        self.assertEqual(self.game_data_calls, [])
        self.assertIn(f'would write {self.root / "check" / overlay.SERVER_GAME_DATA}', said)
        self.assertFalse(check_blobs.exists())
        self.assertNotEqual(self.plan_calls[0][3], str(check_blobs))
        self.assertFalse(Path(self.plan_calls[0][3]).exists())
        # Never over data that is there.
        (self.root / 'other').mkdir()
        (self.root / 'other' / overlay.SERVER_GAME_DATA).write_bytes(b'old')
        with self.assertRaises(SystemExit) as refused:
            self.build(self.root / 'other' / 'y.zip', '--pack-zip', *release, plan=self.release_of())
        self.assertIn('already there', str(refused.exception))
        self.assertFalse((self.root / 'other' / 'y.zip').exists())
        # Without the tool, nothing is written.
        with self.assertRaises(SystemExit) as refused:
            self.build(self.root / 'none' / 'z.zip', '--pack-zip', *release,
                       f'--game-data-tool={self.root / "no-such-tool"}', game_data_tool='real', plan=self.release_of())
        self.assertIn('cargo build --release -p resources-db --bin srp-game-data', str(refused.exception))
        self.assertFalse((self.root / 'none').exists())
        # A ZIP that leaves the db alone: no server data.
        self.build(self.root / 'plain.zip', '--pack-zip', '--no-late-join')
        self.assertEqual(self.game_data_calls, [])

    def test_a_release_the_game_or_the_updater_would_refuse_is_refused(self):
        self.fake_db()
        final_db, blobs = f'--resources-db={self.root / "final.db"}', f'--resources-out={self.root / "blobs"}'
        for options, plan, said in [
            ((final_db,), self.release_of(), 'needs --resources-out'),
            ((final_db, blobs, '--no-launcher', '--no-late-join'), self.release_of(), 'needs the updater'),
            ((final_db, final_db, blobs), self.release_of(), 'given twice'),
            ((blobs, '--no-late-join'), None, 'is for --resources-db'),
            ((final_db, blobs), self.release_of(signer='retail'), 'signed by retail'),
            ((final_db, blobs), self.release_of(signer=None), 'signed by neither key'),
            ((final_db, blobs), self.release_of(claimed=10_000), 'not the head of a resources.db'),
        ]:
            with self.subTest(said=said):
                with self.assertRaises(SystemExit) as refused:
                    self.build(self.root / 'refused.zip', '--pack-zip', *options, plan=plan)
                self.assertIn(said, str(refused.exception))
                self.assertFalse((self.root / 'refused.zip').exists())
                shutil.rmtree(self.root / 'blobs', ignore_errors=True)

    def test_the_default_exe_ships_with_the_skhid_signed_db(self):
        """The default exe starts with no resources.db but one whose table
        Skhid's key signed: a release of it without such a db, or such a db
        with any other exe, would stop every player's game."""
        self.fake_db()
        release = (f'--resources-db={self.root / "final.db"}', f'--resources-out={self.root / "blobs"}')
        cases = [((), None), (('--no-launcher',), None),
                 (release, self.release_of(head=False, tail=b''))] + [
                ((*release, exe), self.release_of()) for exe in ('--no-late-join', '--cert-only',
                                                                 '--cave=hash-mismatch')]
        for options, plan in cases:
            with self.subTest(options=options):
                with self.assertRaises(SystemExit) as refused:
                    self.build(self.root / 'refused.zip', '--pack-zip', *options, plan=plan or ([], {}))
                self.assertIn('file table Skhid signed', str(refused.exception))
                self.assertFalse((self.root / 'refused.zip').exists())

    @unittest.skipIf(os.name == 'nt', 'the stand-in tool is a shell script')
    def test_the_plan_comes_from_srp_game_data(self):
        """release_plan runs `srp-game-data plan` and takes its lines, or
        stops with what it said."""
        tool = self.root / 'srp-game-data'
        tool.write_text('#!/bin/sh\necho "$@" > "$(dirname "$0")/args"\n'
                        'printf "region game/resources.db 1 0 1 a b x\\n"\n')
        tool.chmod(0o755)
        final_db = self.root / 'final.db'
        final_db.write_bytes(b'final')
        lines = overlay.release_plan(str(tool), str(self.game), str(final_db), str(self.root / 'blobs'))
        self.assertEqual(lines, ['region game/resources.db 1 0 1 a b x'])
        self.assertEqual((self.root / 'args').read_text().split(),
                         ['plan', '--original', str(self.game / 'resources.db'), '--final', str(final_db),
                          '--blobs', str(self.root / 'blobs')])
        tool.write_text('#!/bin/sh\necho "a shorter file" >&2\nexit 1\n')
        with self.assertRaises(SystemExit) as refused:
            overlay.release_plan(str(tool), str(self.game), str(final_db), str(self.root / 'blobs'))
        self.assertIn('srp-game-data plan: a shorter file', str(refused.exception))
        tool.write_text('#!/bin/sh\nexit 0\n')
        with self.assertRaises(SystemExit) as refused:
            overlay.release_plan(str(tool), str(self.game), str(final_db), str(self.root / 'blobs'))
        self.assertIn('nothing to change', str(refused.exception))

    def test_existing_release_is_not_overwritten(self):
        archive = self.root / 'release.zip'
        archive.write_bytes(b'existing release')
        with self.assertRaises(SystemExit):
            self.build(archive, '--pack-zip')
        self.assertEqual(archive.read_bytes(), b'existing release')


if __name__ == '__main__':
    unittest.main()
