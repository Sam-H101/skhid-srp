"""Inspect real Win32 crash dialogs under Wine; no game or real browser launch.

Usage: python3 tests/check_localization.py BUILD [SCREENSHOT_DIRECTORY]
Build must contain ui_probe.exe and BugTrap.dll. Use an isolated WINEPREFIX.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess
import tempfile
import zipfile

# Independent expected UI strings, including Cyrillic and Polish diacritics.
LANGUAGES = {
    "english": ("0409", "The game has crashed.", "Open Discord", "Close", "File:", "Folder:", "Windows error", "Could not open your browser."),
    "russian": ("0419", "В игре произошёл сбой.", "Открыть Discord", "Закрыть", "Файл:", "Папка:", "Ошибка Windows", "Не удалось открыть браузер."),
    "polish": ("0415", "Gra uległa awarii.", "Otwórz Discord", "Zamknij", "Plik:", "Folder:", "Błąd systemu Windows", "Nie udało się otworzyć przeglądarki."),
    "ukrainian": ("0422", "У грі стався збій.", "Відкрити Discord", "Закрити", "Файл:", "Папка:", "Помилка Windows", "Не вдалося відкрити браузер."),
    "german": ("0807", "Das Spiel ist abgestürzt.", "Discord öffnen", "Schließen", "Datei:", "Ordner:", "Windows-Fehler", "Der Browser konnte nicht geöffnet werden."),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build", type=Path)
    parser.add_argument("screenshots", type=Path, nargs="?")
    parser.add_argument("--states", nargs="+", default=["success", "raw", "partial", "failure", "browser-failure", "dialog-failure"])
    options = parser.parse_args()
    build = options.build.resolve()
    screenshots = options.screenshots.resolve() if options.screenshots else None
    if screenshots:
        screenshots.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="srp-ui-test-") as temporary:
        root = Path(temporary)
        for filename in ("BugTrap.dll", "ui_probe.exe"):
            shutil.copy2(build / filename, root / filename)
        cases = [(language, state, "0409", [f"-localization={language}"])
                 for language in LANGUAGES
                 for state in options.states]
        # No game option: Windows' UI language, including German regional variant.
        cases += [(language, "success", text[0], []) for language, text in LANGUAGES.items()]
        cases += [("english", "success", "040c", []),
                  ("english", "success", "0419", ["-localization=english"])]
        for language, state, windows, args in cases:
            before = set(root.glob("SRP-crash-*"))
            env = os.environ.copy()
            if screenshots and state == "success" and args:
                env["SRP_UI_SCREENSHOT"] = "Z:" + str(screenshots / f"{language}.bmp")
            result = subprocess.run(["wine", str(root / "ui_probe.exe"), state, windows, *args],
                                    cwd=root, env=env, capture_output=True, timeout=45)
            output = result.stdout.decode("utf-8")
            assert result.returncode == 0 and "PASS localized" in output, (
                language, state, result.returncode, output, result.stderr.decode(errors="replace"))
            controls = {int(line.split(" ", 2)[1]): line.split(" ", 2)[2]
                        for line in output.splitlines() if line.startswith("TEXT ")}
            _, intro, button, close, file_label, folder_label, error, browser_error = LANGUAGES[language]
            if state == "dialog-failure":
                [report] = set(root.glob("SRP-crash-*")) - before
                assert report.name in controls[-1] and report.suffix == ".zip", controls
                assert "discord.gg/Pxa4prXfwu" in controls[-1] and "#bugreports" in controls[-1], controls
                assert button not in controls[-1], "fallback must not refer to a missing button"
                report.unlink()
                print(f"PASS {language}: dialog creation failure", flush=True)
                continue
            assert controls[1030] == intro and controls[1013] == button and controls[2] == close, controls
            assert "#bugreports" in controls[1036] and button in controls[1036], controls
            if state in ("partial", "failure"):
                assert error in controls[1031], controls
            new = set(root.glob("SRP-crash-*")) - before
            if state == "failure":
                assert not new and all(i not in controls for i in (1034, 1035, 1037, 1038)), controls
            else:
                assert controls[1037] == file_label and controls[1038] == folder_label, controls
                [report] = new
                assert controls[1034] == report.name and str(root).replace("/", "\\") in controls[1035], controls
                if state in ("raw", "partial"):
                    assert report.suffix == ".dmp" and report.read_bytes()[:4] == b"MDMP"
                else:
                    assert report.suffix == ".zip"
                    with zipfile.ZipFile(report) as archive:
                        assert archive.testzip() is None
                report.unlink()
            if state == "browser-failure":
                assert controls[-1].startswith(browser_error), controls
                assert "discord.gg/Pxa4prXfwu" in controls[-1] and "#bugreports" in controls[-1]
            print(f"PASS {language}: {state}, {'game language' if args else 'Windows language'}", flush=True)


if __name__ == "__main__":
    main()
