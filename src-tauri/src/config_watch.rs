use log::warn;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{path::Path, sync::mpsc, time::Duration};
use tauri::Manager;

fn concerns_config(event: &Event, path: &Path) -> bool {
    !matches!(event.kind, EventKind::Access(_)) && event.paths.iter().any(|p| p == path)
}

#[cfg(windows)]
fn hold_config_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // Keep sharing reads/writes, but omit FILE_SHARE_DELETE. Explorer and other
    // processes cannot remove/rename the active config. The Store can still
    // open, truncate and write it through its own handles.
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)
    };
    match open() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => drop(file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            open()
        }
        result => result,
    }
}

// Called once during setup. Watch the directory so removal/recreation or an
// editor's atomic rename does not invalidate a watch on the old file handle.
pub fn start(app: tauri::AppHandle) -> Result<(), String> {
    let dir = dirs::config_dir()
        .ok_or("Config directory unavailable")?
        .join(&app.config().tauri.bundle.identifier);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.json");
    #[cfg(windows)]
    {
        // Keep protection even if initializing the optional watcher below fails.
        static FILE_GUARD: once_cell::sync::OnceCell<std::fs::File> =
            once_cell::sync::OnceCell::new();
        if let Err(e) = FILE_GUARD.get_or_try_init(|| hold_config_file(&path)) {
            warn!("Could not protect configuration from deletion: {}", e);
        }
    }
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = tx.send(event);
        },
        notify::Config::default(),
    )
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            // This one watcher and reload worker live for the application's lifetime.
            let _watcher = watcher;
            while let Ok(event) = rx.recv() {
                match event {
                    Ok(event) if concerns_config(&event, &path) => {}
                    Ok(_) => continue,
                    Err(e) => {
                        warn!("Config watch error: {}", e);
                        continue;
                    }
                }
                // Wait for writes to settle before parsing the file. Access events
                // are ignored to avoid a read -> notification -> read loop on Linux.
                while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}
                match crate::config::reload() {
                    Ok(()) => {
                        let _ = app.emit_all("config_file_changed", ());
                    }
                    Err(e) => warn!("Config reload failed; keeping cached settings: {}", e),
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RenameMode};

    #[test]
    fn reloads_on_creation_and_atomic_replacement_but_not_reads_or_other_files() {
        let path = Path::new("config.json");
        assert!(concerns_config(
            &Event::new(EventKind::Create(CreateKind::File)).add_path(path.into()),
            path
        ));
        assert!(concerns_config(
            &Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path("config.tmp".into())
                .add_path(path.into()),
            path
        ));
        assert!(!concerns_config(
            &Event::new(EventKind::Access(AccessKind::Any)).add_path(path.into()),
            path
        ));
        assert!(!concerns_config(
            &Event::new(EventKind::Create(CreateKind::File)).add_path("answers.json".into()),
            path
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_guard_blocks_removal_and_rename_but_allows_read_write() {
        let dir = std::env::temp_dir().join(format!(
            "goc-config-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        let moved = dir.join("moved.json");
        let guard = hold_config_file(&path).unwrap();
        std::fs::write(&path, b"{\"app_theme\":\"dark\"}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"app_theme\":\"dark\"}");
        assert!(std::fs::rename(&path, &moved).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(guard);
        std::fs::rename(&path, &moved).unwrap();
        std::fs::remove_file(moved).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
