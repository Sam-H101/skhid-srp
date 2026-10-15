// SPDX-License-Identifier: GPL-3.0-or-later
//! `srp-game-data extract`: the server's game data of a release
//! ([`resources_db::server`]), from 0.26g's `resources.db` and the players'
//! ZIP with its blobs. `make_client_026g.py` runs it when it makes a release
//! that changes `resources.db`.
//!
//! ```text
//! srp-game-data extract --db <0.26g resources.db> [--zip <players' ZIP> --blobs <folder>] --out <file>
//! srp-game-data check --data <file> [--zip <players' ZIP>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use resources_db::Archive;
use resources_db::server::{check, extract, plan_line};

const USAGE: &str = "usage:
  srp-game-data extract --db <0.26g resources.db> [--zip <players' ZIP> --blobs <folder>] --out <file>
  srp-game-data check --data <file> [--zip <players' ZIP>]";

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = updater_logic::sha256::Sha256::new();
    hasher.update(bytes);
    String::from_utf8_lossy(&updater_logic::sha256::to_hex(&hasher.finish())).into_owned()
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, options) = args.split_first().ok_or(USAGE)?;
    let mut named = std::collections::BTreeMap::new();
    let mut rest = options.iter();
    while let Some(key) = rest.next() {
        let value = rest
            .next()
            .ok_or_else(|| format!("{key} needs a value\n{USAGE}"))?;
        if named.insert(key.as_str(), PathBuf::from(value)).is_some() {
            return Err(format!("{key} is given twice"));
        }
    }
    let mut take = |key: &str| named.remove(key);
    match command.as_str() {
        "extract" => {
            let db = take("--db").ok_or(USAGE)?;
            let out = take("--out").ok_or(USAGE)?;
            let zip = take("--zip");
            let blobs = take("--blobs");
            if zip.is_some() != blobs.is_some() {
                return Err(format!("--zip and --blobs go together\n{USAGE}"));
            }
            if let Some(key) = named.keys().next() {
                return Err(format!("unknown option {key}\n{USAGE}"));
            }
            let blobs = blobs.unwrap_or_default();
            let (bytes, extracted) =
                extract(&db, zip.as_deref(), &blobs).map_err(|e| e.to_string())?;
            let temporary = out.with_extension("db.part");
            std::fs::write(&temporary, &bytes)
                .and_then(|()| std::fs::rename(&temporary, &out))
                .map_err(|cause| format!("{}: {cause}", out.display()))?;
            println!(
                "{}: {} bytes, sha256 {}, {} gameplay files, {} projects, {}",
                out.display(),
                bytes.len(),
                sha256_hex(&bytes),
                extracted.gameplay,
                extracted.projects,
                plan_line(&extracted.provenance)
            );
            Ok(())
        }
        "check" => {
            let data = take("--data").ok_or(USAGE)?;
            let zip = take("--zip");
            if let Some(key) = named.keys().next() {
                return Err(format!("unknown option {key}\n{USAGE}"));
            }
            let file = std::fs::File::open(&data)
                .map_err(|cause| format!("{}: {cause}", data.display()))?;
            let mut archive = Archive::new(file).map_err(|e| e.to_string())?;
            let zip = zip
                .map(|zip| {
                    std::fs::read(&zip).map_err(|cause| format!("{}: {cause}", zip.display()))
                })
                .transpose()?;
            match check(&mut archive, zip.as_deref()).map_err(|e| e.to_string())? {
                Ok(provenance) => {
                    println!("{}: {}", data.display(), plan_line(&provenance));
                    Ok(())
                }
                Err(mismatch) => Err(format!(
                    "{} was made for {}; the ZIP has {}",
                    data.display(),
                    plan_line(&mismatch.made_for),
                    plan_line(&mismatch.zip_has)
                )),
            }
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("srp-game-data: {message}");
            ExitCode::FAILURE
        }
    }
}
