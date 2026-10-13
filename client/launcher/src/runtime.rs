//! OS-specific game start. The launcher itself never runs inside Wine.
use crate::update::{Layout, Result};
#[cfg(target_os = "linux")]
use std::io::Write;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

pub const LANGUAGES: [(&str, &str); 5] = [
    ("English", "english"),
    ("Українська", "ukrainian"),
    ("Русский", "russian"),
    ("Deutsch", "german"),
    ("Polski", "polish"),
];

#[cfg(target_os = "linux")]
const WINE_SCRIPT: &str = include_str!("../../../srp-026g/client-patches/play_skhid.sh");

// Read the generated literal server setting, never source/evaluate shell code.
fn login_server(layout: &Layout) -> Result<Option<String>> {
    let path = layout.place("game/binaries/x86/play_skhid.sh")?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let value = text.lines().find_map(|line| line.strip_prefix("server="));
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(value)
            if value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".:-[]".contains(&c)) =>
        {
            Ok(Some(value.into()))
        }
        Some(_) => Err("The server= line in play_skhid.sh must be a literal host:port.".into()),
        None => Ok(None),
    }
}

pub fn command(layout: &Layout, language: &str, setup_only: bool) -> Result<Command> {
    if !LANGUAGES.iter().any(|(_, value)| *value == language) {
        return Err("Choose a supported game language.".into());
    }
    let game = layout.place("game/binaries/x86/survarium_skhid.exe")?;
    if !game.is_file() {
        return Err("The patched game executable is missing. Check for updates first.".into());
    }
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("bash");
        c.args(["-s", "--"]).stdin(Stdio::piped());
        if setup_only {
            c.arg("setup");
        }
        c.env("SKHID_LOCALIZATION", language);
        c.env("SKHID_GAME_BIN", &layout.bin);
        c.env("SKHID_SERVER", login_server(layout)?.unwrap_or_default());
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        if setup_only {
            return Err("No Wine setup is required on Windows.".into());
        }
        let mut c = Command::new(game);
        c.args([
            format!("-localization={language}"),
            "-mt_allocator=768".into(),
            "-minimum_resources_memory_size=768".into(),
        ]);
        if let Some(server) = login_server(layout)? {
            c.arg(format!("-server={server}"));
        }
        c
    };
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    return Err("This launcher currently supports Linux and Windows.".into());
    cmd.current_dir(&layout.bin);
    Ok(cmd)
}

pub fn run(layout: &Layout, language: &str, setup_only: bool) -> Result<String> {
    crate::update::recover_pending(layout)?;
    // The Linux script takes the same installation lock as the native updater
    // and holds it through setup and the game. Windows has no wrapper.
    #[cfg(target_os = "windows")]
    let _lock = crate::update::Lock::acquire(layout)?;
    let mut cmd = command(layout, language, setup_only)?;
    let path = layout.place("game/binaries/x86/skhid-native.log")?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    cmd.stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(target_os = "windows")]
    cmd.stdin(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("Could not start: {e}"))?;
    #[cfg(target_os = "linux")]
    {
        let result = child
            .stdin
            .take()
            .ok_or("Missing setup script input")?
            .write_all(WINE_SCRIPT.replace("\r\n", "\n").as_bytes());
        if let Err(error) = result {
            let _ = child.wait();
            return Err(format!(
                "Wine setup stopped before reading its script: {error}. See {}",
                path.display()
            ));
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "{} failed ({status}). See {} and skhid-wine.log.",
            if setup_only { "Setup" } else { "Game" },
            path.display()
        ));
    }
    Ok(if setup_only {
        "Wine and DXVK are ready."
    } else {
        "Game closed."
    }
    .into())
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub language: String,
}

pub fn load_language(layout: &Layout) -> String {
    layout
        .place("game/binaries/x86/.skhid-native.json")
        .ok()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .filter(|s| LANGUAGES.iter().any(|(_, v)| *v == s.language))
        .map(|s| s.language)
        .unwrap_or_else(|| "english".into())
}

pub fn save_language(layout: &Layout, language: &str) -> Result<()> {
    let _lock = crate::update::Lock::acquire(layout)?;
    let path = layout.place("game/binaries/x86/.skhid-native.json")?;
    fs::write(
        path,
        serde_json::to_vec(&Settings {
            language: language.into(),
        })
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn initial_folder() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            if bin.file_name() == Some(std::ffi::OsStr::new("x86")) {
                if let Some(game) = bin.parent().and_then(Path::parent) {
                    return game.to_owned();
                }
            }
        }
    }
    std::env::current_dir().unwrap_or_default()
}
