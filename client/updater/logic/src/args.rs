//! A Windows command line cut into its arguments, by the rules of
//! `CommandLineToArgvW` and the C runtime: spaces and tabs part arguments
//! outside double quotes; 2n backslashes before a quote are n backslashes
//! and the quote opens or closes; 2n+1 are n backslashes and a quote
//! character; backslashes elsewhere are themselves.

use alloc::string::String;
use alloc::vec::Vec;

pub fn split(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(c) = characters.next() {
        match c {
            '\\' => {
                let mut count = 1;
                while characters.peek() == Some(&'\\') {
                    characters.next();
                    count += 1;
                }
                started = true;
                if characters.peek() == Some(&'"') {
                    current.extend(core::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        characters.next();
                        current.push('"');
                    }
                } else {
                    current.extend(core::iter::repeat_n('\\', count));
                }
            }
            '"' => {
                started = true;
                // Two quotes inside a quoted part are one quote character.
                if quoted && characters.peek() == Some(&'"') {
                    characters.next();
                    current.push('"');
                } else {
                    quoted = !quoted;
                }
            }
            ' ' | '\t' if !quoted => {
                if started {
                    arguments.push(core::mem::take(&mut current));
                    started = false;
                }
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn the_launchers_command_line() {
        // As the 0.26g launcher builds it: every argument in quotes.
        let line = r#""D:\Games\Survarium\game\binaries\x86\updater_skhid.exe" "update" "\\.\pipe\vostok_updater_pipe-survarium" "0.26g0" "direct" "D:\Games\Survarium" "" "user name""#;
        assert_eq!(
            split(line),
            [
                r"D:\Games\Survarium\game\binaries\x86\updater_skhid.exe",
                "update",
                r"\\.\pipe\vostok_updater_pipe-survarium",
                "0.26g0",
                "direct",
                r"D:\Games\Survarium",
                "",
                "user name",
            ]
        );
    }

    #[test]
    fn the_rules_of_quotes_and_backslashes() {
        assert_eq!(split(""), Vec::<String>::new());
        assert_eq!(split("  a\tb  "), ["a", "b"]);
        assert_eq!(split(r#"a "b c" d"#), ["a", "b c", "d"]);
        assert_eq!(split(r#"a\\b "c\\" d"#), [r"a\\b", r"c\", "d"]);
        assert_eq!(split(r#""a \"quoted\" word""#), [r#"a "quoted" word"#]);
        assert_eq!(split(r#"a\\\"b"#), [r#"a\"b"#]);
        assert_eq!(split(r#""a""b" c"#), [r#"a"b"#, "c"]);
        assert_eq!(split(r#""" x"#), ["", "x"]);
        // A folder with a backslash at its end swallows the quote: what
        // Windows does too.
        assert_eq!(split(r#""C:\dir\" next"#), [r#"C:\dir" next"#]);
    }
}
