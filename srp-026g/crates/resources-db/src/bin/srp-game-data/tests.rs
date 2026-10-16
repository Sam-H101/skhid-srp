// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn parsed(text: &str) -> Result<(PathBuf, Vec<PatchFile>, Option<PathBuf>), String> {
    archives(&args(text))
}

fn refused(text: &str) -> String {
    parsed(text).expect_err(text)
}

/// A game folder of its own under the system's temporary one, removed
/// after.
struct Game(PathBuf);

impl Game {
    fn new(name: &str, files: &[&str]) -> Self {
        let path =
            std::env::temp_dir().join(format!("srp-game-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        for file in files {
            std::fs::write(path.join(file), b"x").unwrap();
        }
        Self(path)
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn db_and_patches_are_named_with_their_urls() {
    let (db, patches, out) = parsed(
        "--db r/resources.db --patch a/resources.db.patch_2=http://h/x=y \
         --patch b/resources.db.patch_1 --patch-url http://h/base/ --out o",
    )
    .unwrap();
    assert_eq!(db, PathBuf::from("r/resources.db"));
    assert_eq!(out, Some(PathBuf::from("o")));
    let given: Vec<(&str, &str)> = patches
        .iter()
        .map(|patch| (patch.path.to_str().unwrap(), patch.url.as_str()))
        .collect();
    // Split at the first `=`; the base URL for a patch without one; the
    // order as given (build puts them in the game's).
    assert_eq!(
        given,
        [
            ("a/resources.db.patch_2", "http://h/x=y"),
            (
                "b/resources.db.patch_1",
                "http://h/base/resources.db.patch_1"
            ),
        ]
    );
    let (_, patches, out) = parsed("--db resources.db").unwrap();
    assert!(patches.is_empty());
    assert_eq!(out, None);
}

#[test]
fn a_game_folder_gives_its_db_and_patches() {
    let game = Game::new(
        "dir",
        &[
            "resources.db",
            "resources.db.patch_3",
            "resources.db.patch_1",
            "resources.db.patch_03",
        ],
    );
    let dir = game.0.display().to_string();
    let (db, patches, _) = parsed(&format!("--dir {dir} --patch-url http://h/p")).unwrap();
    assert_eq!(db, game.0.join("resources.db"));
    let urls: Vec<&str> = patches.iter().map(|patch| patch.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "http://h/p/resources.db.patch_1",
            "http://h/p/resources.db.patch_3"
        ]
    );
    // Every patch of the folder needs a URL.
    assert!(refused(&format!("--dir {dir}")).contains("no URL"));
    // A folder without resources.db.
    let empty = Game::new("empty", &[]);
    assert!(
        refused(&format!(
            "--dir {} --patch-url http://h/p",
            empty.0.display()
        ))
        .contains("no such file")
    );
}

#[test]
fn options_that_do_not_say_one_set_of_archives_are_refused() {
    for (options, why) in [
        ("", "either --dir"),
        ("--patch-url http://h/p", "either --dir"),
        ("--dir d --db resources.db", "either --dir"),
        (
            "--dir d --patch resources.db.patch_1=http://h/x",
            "either --dir",
        ),
        ("--db a --db b", "given twice"),
        ("--dir a --dir b", "given twice"),
        (
            "--db a --patch-url http://h/ --patch-url http://h/",
            "given twice",
        ),
        ("--db a --out o --out p", "given twice"),
        ("--db", "needs a value"),
        ("--db a --patch", "needs a value"),
        ("--db a --other b", "unknown option --other"),
        ("--db a b", "unknown option b"),
        ("--db a --patch resources.db.patch_1", "no URL"),
    ] {
        let message = refused(options);
        assert!(message.contains(why), "{options:?}: {message}");
    }
}
