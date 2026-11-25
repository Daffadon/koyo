use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const LOG_FILE: &str = "koyo.log";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct FileLogger {
    path: PathBuf,
    file: Mutex<File>,
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
        if let Ok(mut file) = self.file.lock() {
            let too_large = file
                .metadata()
                .map(|metadata| metadata.len() > MAX_LOG_BYTES)
                .unwrap_or(false);
            if too_large {
                drop(file);
                rotate_log(&self.path);
                match open_log(&self.path) {
                    Ok(mut reopened) => {
                        write_log_line(&mut reopened, &timestamp, record);
                        if let Ok(mut slot) = self.file.lock() {
                            *slot = reopened;
                        }
                    }
                    Err(error) => {
                        eprintln!("Could not reopen Koyo log after rotation: {error}");
                    }
                }
            } else {
                write_log_line(&mut file, &timestamp, record);
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
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
    log::set_boxed_logger(Box::new(FileLogger {
        path: path.clone(),
        file: Mutex::new(file),
    }))
    .map_err(|error| format!("Could not install the Koyo logger: {error}"))?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}

fn write_log_line(
    file: &mut File,
    timestamp: &chrono::format::DelayedFormat<chrono::format::StrftimeItems<'_>>,
    record: &Record,
) {
    let _ = writeln!(
        file,
        "{timestamp} {:<5} {}: {}",
        record.level(),
        record.target(),
        record.args()
    );
    let _ = file.flush();
}

fn open_log(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))
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
    use super::{MAX_LOG_BYTES, rotate_log};
    use std::fs;

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
        let backtrace = std::backtrace::Backtrace::capture();
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
