//! Native HTTP and filesystem implementation of the existing Skhid update format.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use updater_logic::{manifest, sha256, url, zip};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub struct Layout {
    pub game: PathBuf,
    pub bin: PathBuf,
    pub top: PathBuf,
}

impl Layout {
    pub fn new(path: &Path) -> Result<Self> {
        let game = if path.join("game/binaries/x86").is_dir() {
            path.join("game")
        } else {
            path.to_owned()
        };
        let game = game
            .canonicalize()
            .map_err(|e| format!("Open the game folder: {e}"))?;
        let bin = game.join("binaries/x86");
        if !bin.is_dir() {
            return Err("Select the folder containing binaries/x86, or its parent.".into());
        }
        let top = if game
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("game"))
        {
            game.parent().ok_or("Game folder has no parent")?.to_owned()
        } else {
            game.clone()
        };
        let layout = Self { game, bin, top };
        layout.place("game/binaries/x86/survarium_skhid.exe")?;
        Ok(layout)
    }

    pub fn place(&self, name: &str) -> Result<PathBuf> {
        if !zip::safe_name(name) {
            return Err(format!("Unsafe update filename: {name}"));
        }
        let (root, rest) = match name.strip_prefix("game/") {
            Some(rest) => (&self.game, rest),
            None => (&self.top, name),
        };
        let mut result = root.clone();
        for part in rest.split('/') {
            result.push(part);
            match fs::symlink_metadata(&result) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!(
                        "Update path is a symbolic link: {}",
                        result.display()
                    ));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(result)
    }

    pub fn site(&self) -> Result<String> {
        let path = self.bin.join("updater_skhid.url");
        match fs::read_to_string(path) {
            Ok(value) => validate_site(value.trim()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("https://skhid.link".into()),
            Err(e) => Err(format!("Read update server: {e}")),
        }
    }
}

pub fn validate_site(site: &str) -> Result<String> {
    url::parse(site)
        .ok_or("Use an HTTPS server URL (HTTP is allowed only for localhost tests).")?;
    Ok(site.trim().trim_end_matches('/').to_owned())
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum Progress {
    Checking,
    Download {
        received: u64,
        total: u64,
        bytes_per_second: u64,
    },
    Install,
}

impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Checking => write!(f, "Checking for updates…"),
            Self::Install => write!(f, "Installing verified files…"),
            Self::Download {
                received, total, ..
            } => write!(
                f,
                "Downloading {:.1} / {:.1} MiB",
                *received as f64 / 1048576.0,
                *total as f64 / 1048576.0
            ),
        }
    }
}

pub fn fetch(
    site: &str,
    route: &str,
    limit: u64,
    progress: &mut dyn FnMut(String),
) -> Result<Vec<u8>> {
    fetch_progress(site, route, limit, &mut |p| progress(p.to_string()))
}

fn fetch_progress(
    site: &str,
    route: &str,
    limit: u64,
    progress: &mut dyn FnMut(Progress),
) -> Result<Vec<u8>> {
    let site = validate_site(site)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .into();
    let mut response = agent
        .get(format!("{site}{route}"))
        .call()
        .map_err(|e| format!("Could not download {route}: {e}"))?;
    if response.status() != 200 {
        return Err(format!("Server returned {} for {route}", response.status()));
    }
    if response.body().content_length().is_some_and(|n| n > limit) {
        return Err("Download exceeds the announced size limit.".into());
    }
    let mut reader = response.body_mut().as_reader();
    let mut data = Vec::new();
    let mut buffer = [0; 65536];
    let mut next_notice = 0;
    let started = std::time::Instant::now();
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if data.len() as u64 + n as u64 > limit {
            return Err("Download exceeds the announced size limit.".into());
        }
        data.extend_from_slice(&buffer[..n]);
        if data.len() >= next_notice {
            if route == "/download" {
                progress(Progress::Download {
                    received: data.len() as u64,
                    total: limit,
                    bytes_per_second: (data.len() as f64
                        / started.elapsed().as_secs_f64().max(0.001))
                        as u64,
                });
            }
            next_notice = data.len() + 256 * 1024;
        }
    }
    Ok(data)
}

fn hash_file(path: &Path) -> Result<(u64, [u8; 32])> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = sha256::Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((size, hash.finish()))
}

const STATE: &str = "game/binaries/x86/.skhid-native.state";
const TRANSACTION: &str = ".skhid-native-update";

pub struct Lock(fs::File);
impl Lock {
    pub fn acquire(layout: &Layout) -> Result<Self> {
        let path = layout.place("game/binaries/x86/.skhid-launcher.lock")?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|_| "The game or another launcher is using this installation.".to_string())?;
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Change {
    name: String,
    existed: bool,
    remove: bool,
}

fn transaction_dir(layout: &Layout) -> Result<PathBuf> {
    let path = layout.top.join(TRANSACTION);
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Update staging folder is a symbolic link.".into());
    }
    Ok(path)
}

fn rollback(layout: &Layout, dir: &Path, changes: &[Change]) -> Result<()> {
    for (i, change) in changes.iter().enumerate().rev() {
        let place = layout.place(&change.name)?;
        if change.existed {
            let backup = dir.join(format!("old-{i}"));
            if !backup.is_file() {
                return Err(format!("Missing update backup: {}", backup.display()));
            }
            // An unchanged file may be locked by another process on Windows.
            if hash_file(&place).ok() == Some(hash_file(&backup)?) {
                continue;
            }
            if place.exists() {
                fs::remove_file(&place).map_err(|e| e.to_string())?;
            }
            // Copy, rather than consume, the backup so interrupted recovery is repeatable.
            fs::copy(&backup, &place).map_err(|e| format!("Restore {}: {e}", place.display()))?;
        } else if place.exists() {
            fs::remove_file(&place).map_err(|e| e.to_string())?;
        }
    }
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

fn recover(layout: &Layout) -> Result<()> {
    let dir = transaction_dir(layout)?;
    if !dir.exists() {
        return Ok(());
    }
    let journal = dir.join("journal.json");
    if !journal.exists() || dir.join("committed").exists() {
        return fs::remove_dir_all(dir).map_err(|e| e.to_string());
    }
    let changes: Vec<Change> =
        serde_json::from_slice(&fs::read(journal).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Read pending update journal: {e}"))?;
    rollback(layout, &dir, &changes)
}

/// Finish recovery before starting either a new update or the game.
pub fn recover_pending(layout: &Layout) -> Result<()> {
    let _lock = Lock::acquire(layout)?;
    recover(layout)
}

struct FileUpdate {
    name: String,
    data: Option<Vec<u8>>,
}

fn write_synced(path: &Path, data: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn install(
    layout: &Layout,
    files: Vec<FileUpdate>,
    before_replace: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<usize> {
    let dir = transaction_dir(layout)?;
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let mut changes = Vec::new();
    let staged = (|| {
        for (i, file) in files.iter().enumerate() {
            let place = layout.place(&file.name)?;
            if place.exists() && !place.is_file() {
                return Err(format!("Not a regular file: {}", place.display()));
            }
            if let Some(parent) = place.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let existed = place.exists();
            if existed {
                let backup = dir.join(format!("old-{i}"));
                fs::copy(&place, &backup).map_err(|e| e.to_string())?;
                fs::File::open(&backup)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
                if hash_file(&backup)? != hash_file(&place)? {
                    return Err("Backup verification failed.".into());
                }
            }
            if let Some(data) = &file.data {
                let new = dir.join(format!("new-{i}"));
                write_synced(&new, data)?;
                if hash_file(&new)? != (data.len() as u64, sha256::digest(data)) {
                    return Err("Written update failed verification.".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = if existed {
                        fs::metadata(&place)
                            .map_err(|e| e.to_string())?
                            .permissions()
                            .mode()
                            & 0o777
                    } else if file.name.ends_with(".sh") || data.starts_with(b"\x7fELF") {
                        0o755
                    } else {
                        0o644
                    };
                    fs::set_permissions(&new, fs::Permissions::from_mode(mode))
                        .map_err(|e| e.to_string())?;
                }
            }
            changes.push(Change {
                name: file.name.clone(),
                existed,
                remove: file.data.is_none(),
            });
        }
        write_synced(
            &dir.join("journal.json"),
            &serde_json::to_vec(&changes).map_err(|e| e.to_string())?,
        )?;
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    let apply = (|| {
        for (i, change) in changes.iter().enumerate() {
            before_replace(i)?;
            let place = layout.place(&change.name)?;
            if change.existed {
                // Windows permits renaming a running executable but does not
                // permit overwriting it. Keep the displaced file until cleanup.
                fs::rename(&place, dir.join(format!("displaced-{i}")))
                    .map_err(|e| format!("Move {} aside: {e}", place.display()))?;
            }
            if !change.remove {
                fs::rename(dir.join(format!("new-{i}")), &place)
                    .map_err(|e| format!("Replace {}: {e}", place.display()))?;
            }
        }
        write_synced(&dir.join("committed"), b"1")?;
        Ok(())
    })();
    if let Err(error) = apply {
        rollback(layout, &dir, &changes)
            .map_err(|restore| format!("{error}. Recovery needs attention: {restore}"))?;
        return Err(error);
    }
    // A committed journal lets the next run safely retry cleanup if a scanner
    // temporarily holds a backup on Windows.
    let _ = fs::remove_dir_all(&dir);
    Ok(files.len().saturating_sub(1))
}

fn reserved(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.split('/').any(|part| part.starts_with(".skhid-"))
        || name.ends_with(".skhid-new")
        || name.ends_with(".skhid-old")
}

pub fn check(layout: &Layout, site: &str, progress: &mut dyn FnMut(String)) -> Result<String> {
    check_progress(layout, site, &mut |p| progress(p.to_string()))
}

pub fn check_progress(
    layout: &Layout,
    site: &str,
    progress: &mut dyn FnMut(Progress),
) -> Result<String> {
    let _lock = Lock::acquire(layout)?;
    recover(layout)?;
    progress(Progress::Checking);
    let bytes = fetch_progress(site, "/api/client-files", 16384, progress)?;
    let manifest =
        manifest::parse_manifest(&bytes).map_err(|e| format!("Invalid server manifest: {e:?}"))?;
    let state_path = layout.place(STATE)?;
    let old = fs::read(&state_path)
        .ok()
        .and_then(|data| manifest::parse_installed(&data));
    let same = old
        .as_ref()
        .is_some_and(|state| state.archive == manifest.sha256);
    if let Some(state) = &old {
        if same
            && state.files.iter().all(|file| {
                layout.place(&file.name).ok().is_some_and(|path| {
                    path.is_file()
                        && (manifest::player_may_edit(&file.name)
                            || hash_file(&path).ok() == Some((file.size, file.sha256)))
                })
            })
        {
            return Ok("Your game files are up to date.".into());
        }
    }
    progress(Progress::Download {
        received: 0,
        total: manifest.size,
        bytes_per_second: 0,
    });
    let archive = fetch_progress(site, "/download", manifest.size, progress)?;
    if archive.len() as u64 != manifest.size || sha256::digest(&archive) != manifest.sha256 {
        return Err(
            "Download size or SHA-256 does not match the server manifest. Nothing installed."
                .into(),
        );
    }
    let entries = zip::entries(&archive).map_err(|e| format!("Invalid update ZIP: {e:?}"))?;
    let mut files = Vec::new();
    let mut installed = Vec::new();
    let mut destinations = std::collections::HashSet::new();
    // Expand and CRC-check every entry before staging or replacing any file.
    for entry in entries {
        if reserved(entry.name) {
            return Err("Archive contains a reserved launcher filename.".into());
        }
        let path = layout.place(entry.name)?;
        if !destinations.insert(path.clone()) {
            return Err("Archive paths resolve to the same file.".into());
        }
        let data = entry
            .read()
            .map_err(|e| format!("Damaged ZIP entry {}: {e:?}", entry.name))?;
        let hash = sha256::digest(&data);
        installed.push(manifest::InstalledFile {
            name: entry.name.into(),
            size: data.len() as u64,
            sha256: hash,
        });
        if hash_file(&path).ok() == Some((data.len() as u64, hash))
            || (same && manifest::player_may_edit(entry.name) && path.is_file())
        {
            continue;
        }
        files.push(FileUpdate {
            name: entry.name.into(),
            data: Some(data),
        });
    }
    if installed.is_empty() {
        return Err("The update contains no files.".into());
    }
    if let Some(old) = old {
        for file in old.files {
            if reserved(&file.name) {
                return Err("Installed state contains a reserved filename.".into());
            }
            if installed.iter().any(|new| new.name == file.name) {
                continue;
            }
            let path = layout.place(&file.name)?;
            if destinations.contains(&path) {
                continue;
            }
            if hash_file(&path).ok() == Some((file.size, file.sha256)) {
                files.push(FileUpdate {
                    name: file.name,
                    data: None,
                });
            }
        }
    }
    let state = manifest::Installed {
        archive: manifest.sha256,
        files: installed,
    };
    files.push(FileUpdate {
        name: STATE.into(),
        data: Some(manifest::write_installed(&state)),
    });
    progress(Progress::Install);
    let count = install(layout, files, &mut |_| Ok(()))?;
    Ok(format!("Update complete ({count} files changed)."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: PathBuf,
        layout: Layout,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "skhid-native-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("game/binaries/x86")).unwrap();
            let layout = Layout::new(&root).unwrap();
            Self { root, layout }
        }
        fn write(&self, name: &str, data: &[u8]) {
            let path = self.layout.place(name).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn change(name: &str, data: Option<&[u8]>) -> FileUpdate {
        FileUpdate {
            name: name.into(),
            data: data.map(Vec::from),
        }
    }

    #[test]
    fn failure_undoes_replacements_deletions_and_new_files() {
        let f = Fixture::new();
        f.write("game/a", b"original");
        f.write("game/b", b"remove me");
        let files = vec![
            change("game/a", Some(b"new")),
            change("game/b", None),
            change("game/c", Some(b"created")),
            change(STATE, Some(b"state")),
        ];
        let error = install(&f.layout, files, &mut |i| {
            if i == 3 {
                Err("injected rename failure".into())
            } else {
                Ok(())
            }
        });
        assert!(error.unwrap_err().contains("injected"));
        assert_eq!(fs::read(f.layout.game.join("a")).unwrap(), b"original");
        assert_eq!(fs::read(f.layout.game.join("b")).unwrap(), b"remove me");
        assert!(!f.layout.game.join("c").exists());
        assert!(!f.layout.top.join(TRANSACTION).exists());
    }
    #[test]
    fn interrupted_transaction_is_recovered_before_next_update() {
        let f = Fixture::new();
        f.write("game/a", b"new bytes from interrupted update");
        f.write("game/b", b"created");
        let dir = transaction_dir(&f.layout).unwrap();
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("old-0"), b"original").unwrap();
        let journal = vec![
            Change {
                name: "game/a".into(),
                existed: true,
                remove: false,
            },
            Change {
                name: "game/b".into(),
                existed: false,
                remove: false,
            },
        ];
        fs::write(
            dir.join("journal.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        recover(&f.layout).unwrap();
        assert_eq!(fs::read(f.layout.game.join("a")).unwrap(), b"original");
        assert!(!f.layout.game.join("b").exists());
    }
    #[test]
    fn committed_transaction_is_cleaned_without_undo() {
        let f = Fixture::new();
        f.write("game/a", b"new");
        let dir = transaction_dir(&f.layout).unwrap();
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("committed"), b"1").unwrap();
        recover(&f.layout).unwrap();
        assert_eq!(fs::read(f.layout.game.join("a")).unwrap(), b"new");
    }
    #[test]
    fn lock_prevents_two_launchers_mutating_one_installation() {
        let f = Fixture::new();
        let guard = Lock::acquire(&f.layout).unwrap();
        assert!(Lock::acquire(&f.layout).is_err());
        drop(guard);
        assert!(Lock::acquire(&f.layout).is_ok());
    }
    #[test]
    fn invalid_paths_and_remote_plain_http_are_rejected() {
        let f = Fixture::new();
        for path in [
            "../outside",
            "game/../../outside",
            "/absolute",
            "game/C:/file",
        ] {
            assert!(f.layout.place(path).is_err());
        }
        assert!(validate_site("http://example.com").is_err());
        assert!(validate_site("https://example.com").is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn updates_do_not_follow_links_outside_installation() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(&f.root, f.layout.game.join("linked")).unwrap();
        assert!(f.layout.place("game/linked/anything").is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_passes_language_as_literal_environment_and_uses_script() {
        let f = Fixture::new();
        f.write("game/binaries/x86/survarium_skhid.exe", b"fixture");
        f.write("game/binaries/x86/play_skhid.sh", b"fixture");
        let cmd = crate::runtime::command(&f.layout, "polish", true).unwrap();
        assert_eq!(cmd.get_program(), "bash");
        assert_eq!(cmd.get_args().last().unwrap(), "setup");
        assert!(
            cmd.get_envs().any(
                |(k, v)| k == "SKHID_LOCALIZATION" && v == Some(std::ffi::OsStr::new("polish"))
            )
        );
        assert!(crate::runtime::command(&f.layout, "english; touch /tmp/no", false).is_err());
    }
}
