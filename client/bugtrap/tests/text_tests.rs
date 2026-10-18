#[path = "../src/crash_text.rs"]
mod crash_text;
#[path = "../src/notice_text.rs"]
mod notice_text;

use notice_text::Language::*;

#[test]
fn game_language_takes_precedence_over_windows() {
    for (line, windows, expected) in [
        ("game.exe -localization=russian", 0x0415, Russian),
        ("game.exe -localization=polish", 0x0419, Polish),
        ("game.exe -localization=english", 0x0419, English),
        ("game.exe -localization=ukrainian", 0x0419, Ukrainian),
        ("game.exe -localization=german", 0x0415, German),
        (
            r#""C:\Games\my game.exe" "-localization=polish""#,
            0x0409,
            Polish,
        ),
        (r#"game.exe -localization="russian""#, 0x0409, Russian),
        (
            "game.exe\t-localization=polish\t-server=test",
            0x0409,
            Polish,
        ),
        ("game.exe", 0x0419, Russian),
        ("game.exe", 0x0415, Polish),
        ("game.exe", 0x0422, Ukrainian),
        ("game.exe", 0x0807, German),
        ("game.exe", 0x040c, English),
        ("game.exe -localization=unknown", 0x0415, Polish),
        ("game.exe -localization=polish-extra", 0x0409, English),
        ("game.exe -other-localization=polish", 0x0409, English),
        (
            r#""C:\Games\-localization=polish game.exe""#,
            0x0409,
            English,
        ),
    ] {
        let wide: Vec<u16> = line.encode_utf16().collect();
        assert_eq!(crash_text::language(&wide, windows), expected, "{line}");
    }
}

#[test]
fn every_report_state_has_localized_instructions_and_fits_its_buffers() {
    for language in [English, Russian, Polish, Ukrainian, German] {
        let t = crash_text::text(language);
        for s in [t.instructions, t.no_file, t.manual] {
            assert!(s.contains("#bugreports"), "{language:?}");
        }
        assert!(t.instructions.contains(t.discord));
        assert!(t.no_file.contains(t.discord));
        assert!(
            t.browser_failed.encode_utf16().count() + t.manual.encode_utf16().count() + 4 < 512
        );
        assert!(t.manual.contains("discord.gg/Pxa4prXfwu") && t.manual.contains("#bugreports"));
        assert!(!t.manual.contains(t.discord));
        assert!(t.title.encode_utf16().count() < 128);
        assert!(t.discord.encode_utf16().count() < 64);
        for s in [
            t.intro, t.saved, t.partial, t.failed, t.error, t.file, t.folder, t.close,
        ] {
            assert!(!s.is_empty(), "{language:?}");
        }
    }
}
