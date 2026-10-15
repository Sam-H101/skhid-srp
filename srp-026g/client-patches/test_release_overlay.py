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
import zipfile
import zlib

import make_client_026g as overlay

# A program that writes down how it was started (client/updater/tests).
STAND_IN = Path(overlay.HERE, '..', '..', 'client', 'updater', 'tests', 'fake_exe', 'target',
                'i686-pc-windows-msvc', 'release', 'fake-exe.exe').resolve()


def forged(original, text):
    """Bytes of the original's length that start with `text` and end in four
    bytes chosen so that the game CRC is the original's (a CRC from 0 with
    no final xor is linear in the bits of a message of one length)."""
    def crc0(data):
        return ~zlib.crc32(data, 0xFFFFFFFF) & 0xFFFFFFFF
    body = text + original[len(text):-4]
    need = crc0(body + b'\0' * 4) ^ crc0(original)
    effects = []
    for bit in range(32):
        tail = (1 << bit).to_bytes(4, 'little')
        effects.append(crc0(bytes(len(body)) + tail))
    # Gaussian elimination over GF(2): which tail bits give `need`.
    basis = {}
    for bit, effect in enumerate(effects):
        chosen = 1 << bit
        while effect:
            top = effect.bit_length() - 1
            if top not in basis:
                basis[top] = (effect, chosen)
                break
            effect ^= basis[top][0]
            chosen ^= basis[top][1]
    tail = 0
    while need:
        top = need.bit_length() - 1
        need ^= basis[top][0]
        tail ^= basis[top][1]
    return body + tail.to_bytes(4, 'little')


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
             for blank in ('', 'blank-') for code in ('en', 'ru', 'ua', 'de', 'pl')}

    def build(self, output, *options, game_data_tool=None):
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
        with mock.patch.object(overlay.P, 'build', return_value=(b'patched fixture', 'test patch')) as patcher:
            with launcher as self.launcher, mock.patch('sys.argv', args), tool, made, \
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
        self.build(archive, '--pack-zip', '--no-launcher')
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

    # --- resource regions ---------------------------------------------------

    def fake_db(self):
        """A small resources.db with two known entries in it, and the table
        of regions that points at them."""
        db = bytearray(os.urandom(4096))
        lobby, login = bytes(db[1000:1000 + 700]), bytes(db[2500:2500 + 300])
        (self.game / 'resources.db').write_bytes(bytes(db))
        regions = {'lobby_menu': (1000, 700, hashlib.sha256(lobby).hexdigest()),
                   'login_menu': (2500, 300, hashlib.sha256(login).hexdigest())}
        return bytes(db), regions

    def entry(self, original, name, crc=True):
        """A changed entry of the original's size, with its game CRC when `crc`."""
        changed = forged(original, b'changed ' + name.encode()) if crc else b'x' * len(original)
        path = self.root / f'{name}.bin'
        path.write_bytes(changed)
        return path, changed

    def test_a_zip_that_changes_the_db_comes_with_the_servers_data(self):
        db, regions = self.fake_db()
        lobby_path, _ = self.entry(db[1000:1700], 'lobby')
        blobs = self.root / 'blobs'
        out = self.root / 'release'
        archive = out / 'Survarium-0.26g-srp-files.zip'
        region = f'--resource-region=lobby_menu={lobby_path}'
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            self.build(archive, '--pack-zip', region, f'--resources-out={blobs}')
            # Made from the game's db, the ZIP and its blobs, beside the ZIP.
            self.assertEqual(self.game_data_calls, [(
                overlay.GAME_DATA_TOOL, str(self.game), str(archive), str(blobs),
                str(out / overlay.SERVER_GAME_DATA))])
            # --check says so and makes nothing.
            _, said = self.build(self.root / 'check' / 'x.zip', '--pack-zip', '--check', region,
                                 f'--resources-out={blobs}')
            self.assertEqual(self.game_data_calls, [])
            self.assertIn(f'would write {self.root / "check" / overlay.SERVER_GAME_DATA}', said)
            # Never over data that is there.
            (self.root / 'other').mkdir()
            (self.root / 'other' / overlay.SERVER_GAME_DATA).write_bytes(b'old')
            with self.assertRaises(SystemExit) as refused:
                self.build(self.root / 'other' / 'y.zip', '--pack-zip', region, f'--resources-out={blobs}')
            self.assertIn('already there', str(refused.exception))
            self.assertFalse((self.root / 'other' / 'y.zip').exists())
            # Without the tool, nothing is written.
            with self.assertRaises(SystemExit) as refused:
                self.build(self.root / 'none' / 'z.zip', '--pack-zip', region, f'--resources-out={blobs}',
                           f'--game-data-tool={self.root / "no-such-tool"}', game_data_tool='real')
            self.assertIn('cargo build --release -p resources-db --bin srp-game-data', str(refused.exception))
            self.assertFalse((self.root / 'none').exists())
        # A ZIP that leaves the db alone: no server data.
        self.build(self.root / 'plain.zip', '--pack-zip', '--no-late-join')
        self.assertEqual(self.game_data_calls, [])

    def test_resource_regions_add_the_plan_and_the_blobs(self):
        db, regions = self.fake_db()
        lobby, login = db[1000:1700], db[2500:2800]
        lobby_path, lobby_new = self.entry(lobby, 'lobby')
        login_path, login_new = self.entry(login, 'login')
        self.assertEqual(overlay.game_crc(lobby_new), overlay.game_crc(lobby))
        self.assertNotEqual(lobby_new, lobby)
        blobs = self.root / 'blobs'
        archive = self.root / 'regions.zip'
        sha = lambda data: hashlib.sha256(data).hexdigest()  # noqa: E731
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            # The options in any order; the plan in the order of the db.
            self.build(archive, '--pack-zip', f'--resource-region=login_menu={login_path}',
                       f'--resource-region=lobby_menu={lobby_path}', f'--resources-out={blobs}')
        with zipfile.ZipFile(archive) as z:
            self.assertEqual(set(z.namelist()), self.names | {overlay.RESOURCES_PLAN})
            plan = z.read(overlay.RESOURCES_PLAN)
        self.assertEqual(plan, (
            'skhid-resources 1\r\n'
            f'region game/resources.db 4096 1000 700 {sha(lobby)} {sha(lobby_new)} lobby_menu\r\n'
            f'region game/resources.db 4096 2500 300 {sha(login)} {sha(login_new)} login_menu\r\n'
            'end\r\n').encode())
        # The targets and the originals, by their sha256, for the server.
        self.assertEqual({p.name: p.read_bytes() for p in blobs.iterdir()},
                         {sha(data): data for data in (lobby, lobby_new, login, login_new)})
        self.assertEqual((self.game / 'resources.db').read_bytes(), db, 'the game is only read')
        # Again into the same folder: the blobs there are the same ones.
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            self.build(self.root / 'again.zip', '--pack-zip', f'--resource-region=lobby_menu={lobby_path}',
                       f'--resources-out={blobs}')
        self.assertEqual(len(list(blobs.iterdir())), 4)
        # The same in the real table: the three entries of build 2777's db,
        # the settings menu's ending where the lobby's begins.
        self.assertEqual(overlay.RESOURCE_REGIONS['main_menu'][:2], (17970448, 1194596))
        self.assertEqual(overlay.RESOURCE_REGIONS['lobby_menu'][:2], (19165044, 38985769))
        self.assertEqual(overlay.RESOURCE_REGIONS['login_menu'][:2], (58150813, 3508744))
        spans = sorted(entry[:2] for entry in overlay.RESOURCE_REGIONS.values())
        self.assertTrue(all(a + n <= b for (a, n), (b, _) in zip(spans, spans[1:])))

    def test_an_entry_the_game_or_the_updater_would_refuse_is_refused(self):
        db, regions = self.fake_db()
        lobby = db[1000:1700]
        good, _ = self.entry(lobby, 'good')
        blobs = f'--resources-out={self.root / "blobs"}'
        short = self.root / 'short.bin'
        short.write_bytes(good.read_bytes()[:-1])
        bad_crc, _ = self.entry(lobby, 'bad', crc=False)
        for options, said in [
            ((f'--resource-region=lobby_menu={short}', blobs), 'it must be the same size'),
            ((f'--resource-region=lobby_menu={bad_crc}', blobs), 'the game would refuse it'),
            ((f'--resource-region=other_menu={good}', blobs), 'with a name of'),
            ((f'--resource-region=lobby_menu={good}',), 'needs --resources-out'),
            ((f'--resource-region=lobby_menu={good}', blobs, '--no-launcher'), 'needs the updater'),
            ((f'--resource-region=lobby_menu={good}', f'--resource-region=lobby_menu={good}', blobs), 'twice'),
            ((blobs,), 'is for --resource-region'),
        ]:
            with self.subTest(said=said), mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
                with self.assertRaises(SystemExit) as refused:
                    self.build(self.root / 'refused.zip', '--pack-zip', *options)
                self.assertIn(said, str(refused.exception))
                self.assertFalse((self.root / 'refused.zip').exists())
                self.assertFalse((self.root / 'blobs').exists())
        # The original entry itself changes nothing.
        original = self.root / 'original.bin'
        original.write_bytes(lobby)
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            with self.assertRaises(SystemExit) as refused:
                self.build(self.root / 'refused.zip', '--pack-zip', f'--resource-region=lobby_menu={original}', blobs)
        self.assertIn('it is the original', str(refused.exception))
        # A db whose entry is not the original one (another build, or changed).
        (self.game / 'resources.db').write_bytes(db[:1000] + b'y' * 700 + db[1700:])
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            with self.assertRaises(SystemExit) as refused:
                self.build(self.root / 'refused.zip', '--pack-zip', f'--resource-region=lobby_menu={good}', blobs)
        self.assertIn('is not the original one', str(refused.exception))

    def grow_manifest(self, db, head_size=500, tail=b'new entries ' * 100, **changes):
        """A grow manifest as resource-porter writes it: a new head (the
        file table) for the db's first bytes, and a tail."""
        import json
        sha = lambda data: hashlib.sha256(data).hexdigest()  # noqa: E731
        head = b'new table ' + db[10:head_size]
        (self.root / 'head.bin').write_bytes(head)
        (self.root / 'tail.bin').write_bytes(tail)
        manifest = {
            'format': 'skhid-db-grow 1', 'path': 'game/resources.db',
            'original_size': len(db), 'new_size': len(db) + len(tail),
            'head': {'name': 'file_table', 'offset': 0, 'size': head_size, 'file': 'head.bin',
                     'original_sha256': sha(db[:head_size]), 'target_sha256': sha(head)},
            'tail': {'name': 'new_items', 'file': 'tail.bin', 'sha256': sha(tail)},
        }
        for key, value in changes.items():
            part, _, field = key.partition('__')
            if field:
                manifest[part][field] = value
            else:
                manifest[part] = value
        path = self.root / 'grow.json'
        path.write_text(json.dumps(manifest))
        return path, head, tail

    def test_a_grow_manifest_adds_a_required_plan(self):
        db, regions = self.fake_db()
        sha = lambda data: hashlib.sha256(data).hexdigest()  # noqa: E731
        manifest, head, tail = self.grow_manifest(db)
        lobby = db[1000:1700]
        lobby_path, lobby_new = self.entry(lobby, 'lobby')
        blobs = self.root / 'blobs'
        archive = self.root / 'grow.zip'
        with mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
            self.build(archive, '--pack-zip', f'--resource-grow={manifest}',
                       f'--resource-region=lobby_menu={lobby_path}', f'--resources-out={blobs}')
        with zipfile.ZipFile(archive) as z:
            plan = z.read(overlay.RESOURCES_PLAN)
        new_size = 4096 + len(tail)
        self.assertEqual(plan, (
            'skhid-resources 2\r\n'
            f'region game/resources.db 4096 0 500 {sha(db[:500])} {sha(head)} file_table\r\n'
            f'region game/resources.db 4096 1000 700 {sha(lobby)} {sha(lobby_new)} lobby_menu\r\n'
            f'tail game/resources.db 4096 {new_size} {sha(tail)} new_items file_table\r\n'
            'end\r\n').encode())
        self.assertEqual({p.name: p.read_bytes() for p in blobs.iterdir()},
                         {sha(data): data for data in (db[:500], head, tail, lobby, lobby_new)})
        self.assertEqual((self.game / 'resources.db').read_bytes(), db, 'the game is only read')
        # Without a tail the plan stays format 1 (the updaters out there).
        self.assertTrue(overlay.resources_plan(['region game/resources.db 1 0 1 a b x']).startswith(
            b'skhid-resources 1\r\n'))

    def test_a_grow_manifest_that_does_not_fit_is_refused(self):
        db, regions = self.fake_db()
        blobs = f'--resources-out={self.root / "blobs"}'
        for changes, said in [
            ({'format': 'other'}, 'manifest'),
            ({'path': 'game/other.db'}, 'only game/resources.db can be grown'),
            ({'original_size': 4095}, 'the manifest grows a db of 4095'),
            ({'new_size': 4096}, 'do not fit'),
            ({'new_size': 4097}, 'the db grows by 1'),
            ({'head__size': 499}, 'the head has 499'),
            ({'head__offset': 900}, 'overlaps the region lobby_menu'),
            ({'head__original_sha256': '0' * 64}, 'not the build it was made for'),
            ({'head__target_sha256': '0' * 64}, "not the manifest's head"),
            ({'tail__sha256': '0' * 64}, "not the manifest's tail"),
            ({'tail__name': 'file_table'}, 'are taken'),
            ({'tail__name': 'lobby_menu'}, 'are taken'),
            ({'head__name': 'a b'}, 'is not a plan name'),
            ({'head': None}, 'incomplete'),
        ]:
            with self.subTest(said=said), mock.patch.dict(overlay.RESOURCE_REGIONS, regions, clear=True):
                manifest, _, _ = self.grow_manifest(db, **changes)
                lobby_path, _ = self.entry(db[1000:1700], 'lobby')
                with self.assertRaises(SystemExit) as refused:
                    self.build(self.root / 'refused.zip', '--pack-zip', f'--resource-grow={manifest}',
                               f'--resource-region=lobby_menu={lobby_path}', blobs)
                self.assertIn(said, str(refused.exception))
                self.assertFalse((self.root / 'refused.zip').exists())
                self.assertFalse((self.root / 'blobs').exists())
        manifest, _, _ = self.grow_manifest(db)
        for options, said in [
            ((f'--resource-grow={manifest}',), 'needs --resources-out'),
            ((f'--resource-grow={manifest}', blobs, '--no-launcher'), 'needs the updater'),
            ((f'--resource-grow={manifest}', f'--resource-grow={manifest}', blobs), 'twice'),
        ]:
            with self.subTest(said=said):
                with self.assertRaises(SystemExit) as refused:
                    self.build(self.root / 'refused.zip', '--pack-zip', *options)
                self.assertIn(said, str(refused.exception))

    def test_the_game_crc_is_the_games_formula(self):
        # movie_db.py's crc0 and game_hash, by their definition: CRC-32 of
        # the reflected polynomial from 0, no final xor, bits reversed.
        def slow(data):
            crc = 0
            for byte in data:
                crc ^= byte
                for _ in range(8):
                    crc = (crc >> 1) ^ (0xEDB88320 if crc & 1 else 0)
            return int(f'{crc:032b}'[::-1], 2)
        for data in (b'', b'a', b'abc', os.urandom(1000)):
            self.assertEqual(overlay.game_crc(data), slow(data))

    def test_existing_release_is_not_overwritten(self):
        archive = self.root / 'release.zip'
        archive.write_bytes(b'existing release')
        with self.assertRaises(SystemExit):
            self.build(archive, '--pack-zip')
        self.assertEqual(archive.read_bytes(), b'existing release')


if __name__ == '__main__':
    unittest.main()
