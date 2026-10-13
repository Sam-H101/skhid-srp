"""Check player-folder/ZIP contents without distributing or needing a game binary."""
import contextlib
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

import make_client_026g as overlay

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
            'game/binaries/x86/updater_skhid.url', 'Skhid.bat', 'Skhid.sh',
        } | {f'game/binaries/x86/launcher_skhid/index-{blank}{code}.html'
             for blank in ('', 'blank-') for code in ('en', 'ru', 'ua', 'de', 'pl')}

    def build(self, output, *options):
        args = ['make_client_026g.py', str(self.game), str(output),
                '--server=release.example:2234', '--localization=polish',
                '--bugtrap=' + str(self.dll), '--updater=' + str(self.updater), *options]
        # Binary patching has its own disassembly suite. Exercise every other
        # part of the real release builder, including source reads and ZIP I/O.
        launcher = mock.patch.object(
            overlay.L, 'build', return_value=(b'patched launcher fixture', 'test launcher'))
        with mock.patch.object(overlay.P, 'build', return_value=(b'patched fixture', 'test patch')) as patcher:
            with launcher as self.launcher, mock.patch('sys.argv', args), \
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

    def test_optional_native_launcher_is_shipped_executable(self):
        native = self.root / 'skhid-launcher'
        data = bytearray(64)
        data[:6] = b'\x7fELF\x02\x01'
        data[16:18] = (3).to_bytes(2, 'little')
        data[18:20] = (62).to_bytes(2, 'little')
        native.write_bytes(data)
        archive = self.root / 'native.zip'
        self.build(archive, '--pack-zip', '--native-launcher=' + str(native))
        with zipfile.ZipFile(archive) as z:
            entry = z.getinfo('game/binaries/x86/skhid-launcher')
            self.assertEqual(stat.S_IMODE(entry.external_attr >> 16), 0o755)
            self.assertEqual(z.read(entry), data)
            self.assertIn(b'run bash Skhid.sh to open the native launcher', z.read('README.txt'))
            self.assertNotIn(b'Skhid.sh (the game)', z.read('README.txt'))
        native.write_bytes(b'not an ELF executable')
        with self.assertRaises(SystemExit):
            self.build(self.root / 'invalid', '--native-launcher=' + str(native))

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

    def test_certificate_only_launcher_has_an_explicit_server(self):
        archive = self.root / 'certificate.zip'
        call, _ = self.build(archive, '--pack-zip', '--cert-only')
        self.assertEqual(call.args[1:], ('cert-only', None, None))
        # The game's launcher passes the address too: this exe has none inside.
        self.assertEqual(self.launcher.call_args.kwargs['arguments'],
                         '-server=release.example:2234 -mt_allocator=768 -minimum_resources_memory_size=768')
        with zipfile.ZipFile(archive) as z:
            self.assertIn(b'server=release.example:2234\n', z.read('game/binaries/x86/play_skhid.sh'))

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

    def test_existing_release_is_not_overwritten(self):
        archive = self.root / 'release.zip'
        archive.write_bytes(b'existing release')
        with self.assertRaises(SystemExit):
            self.build(archive, '--pack-zip')
        self.assertEqual(archive.read_bytes(), b'existing release')


if __name__ == '__main__':
    unittest.main()
