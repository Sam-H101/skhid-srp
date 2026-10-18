#[path = "../src/crash_text.rs"]
mod crash_text;
#[path = "../src/notice_text.rs"]
mod notice_text;

use notice_text::Language::*;

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
