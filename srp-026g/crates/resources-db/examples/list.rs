// SPDX-License-Identifier: GPL-3.0-or-later
//! `cargo run --example list -- <resources.db> [prefix]`: path, size and
//! offset of every file (under the prefix).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: list <resources.db> [prefix]")?;
    let prefix = args.next().unwrap_or_default();
    let archive = resources_db::Archive::new(std::fs::File::open(path)?)?;
    for (path, entry) in archive.files_under(&prefix) {
        println!("{path}\t{}\t{}", entry.size, entry.offset);
    }
    for folder in archive.empty_folders() {
        eprintln!("empty folder {}", String::from_utf8_lossy(folder));
    }
    Ok(())
}
