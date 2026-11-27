use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const LOG_FILE: &str = "koyo.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct LogState {
    file: Option<File>,
    bytes: u64,
}

struct FileLogger {
    path: PathBuf,
    state: Mutex<LogState>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Third-party INFO/WARN chatter (DBus, compositor, GPU probing) belongs
        // on stderr, not in a support log. Keep the file focused on Koyo.
        metadata.level() <= Level::Info
            && (metadata.target().starts_with("koyo") || metadata.target().starts_with("koyo_core"))
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        if let Ok(mut state) = self.state.lock() {
            if state.bytes > MAX_LOG_BYTES {
                // Close the handle while holding the mutex so Windows rename
                // succeeds and concurrent log calls cannot interleave rotation.
                drop(state.file.take());
                rotate_log(&self.path);
                state.bytes = 0;
            }
            if state.file.is_none() {
                match open_log(&self.path) {
                    Ok(file) => {
                        state.bytes = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                        state.file = Some(file);
                    }
                    Err(error) => {
                        eprintln!("Could not reopen Koyo log after rotation: {error}");
                        return;
                    }
                }
            }
            if let Some(file) = state.file.as_mut() {
                let written = write_log_line(file, &timestamp, record);
                state.bytes = state.bytes.saturating_add(written as u64);
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock()
            && let Some(file) = state.file.as_mut()
        {
            let _ = file.flush();
        }
    }
}

pub(crate) fn log_path() -> PathBuf {
    koyo_core::storage::default_config_dir().join(LOG_FILE)
}

pub(crate) fn config_path() -> PathBuf {
    koyo_core::storage::default_config_dir()
}

pub(crate) fn init_logging() -> Result<(), String> {
    let path = log_path();
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid Koyo log path.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the Koyo log directory: {error}"))?;
    rotate_log(&path);
    let file = open_log(&path)?;
    let bytes = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    log::set_boxed_logger(Box::new(FileLogger {
        path: path.clone(),
        state: Mutex::new(LogState {
            file: Some(file),
            bytes,
        }),
    }))
    .map_err(|error| format!("Could not install the Koyo logger: {error}"))?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}

fn write_log_line(
    file: &mut File,
    timestamp: &chrono::format::DelayedFormat<chrono::format::StrftimeItems<'_>>,
    record: &Record,
) -> usize {
    let line = format!(
        "{timestamp} {:<5} {}: {}\n",
        record.level(),
        record.target(),
        record.args()
    );
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
    line.len()
}

fn open_log(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    Ok(file)
}

fn rotate_log(path: &Path) {
    let too_large = fs::metadata(path)
        .map(|metadata| metadata.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if !too_large {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(path, old);
}

#[cfg(test)]
mod tests {
    use super::{FileLogger, LogState, MAX_LOG_BYTES, open_log, rotate_log};
    use log::{Level, Log, Record};
    use std::fs;
    use std::sync::Mutex;

    #[test]
    fn oversized_log_rotates_once() {
        let directory =
            std::env::temp_dir().join(format!("koyo-log-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create log temp dir");
        let path = directory.join("koyo.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).expect("seed log");
        rotate_log(&path);
        assert!(!path.exists());
        assert!(path.with_extension("log.1").exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn logger_rotates_under_lock_and_writes_new_file() {
        let directory =
            std::env::temp_dir().join(format!("koyo-logger-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create log temp dir");
        let path = directory.join("koyo.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).expect("seed log");
        let file = open_log(&path).expect("open seeded log");
        let bytes = file.metadata().expect("metadata").len();
        let logger = FileLogger {
            path: path.clone(),
            state: Mutex::new(LogState {
                file: Some(file),
                bytes,
            }),
        };
        logger.log(
            &Record::builder()
                .args(format_args!("after rotation"))
                .level(Level::Info)
                .target("koyo::test")
                .build(),
        );
        assert!(path.with_extension("log.1").exists());
        let fresh = fs::read_to_string(&path).expect("read fresh log");
        assert!(fresh.contains("after rotation"));
        let _ = fs::remove_dir_all(directory);
    }
}

pub(crate) fn install_panic_hook() {
    let path = log_path();
    std::panic::set_hook(Box::new(move |info| {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let line = format!("{timestamp} ERROR koyo::panic: {payload} at {location}\n{backtrace}\n");
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        if let Ok(mut file) = options.open(&path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        eprintln!("{line}");
    }));
}
