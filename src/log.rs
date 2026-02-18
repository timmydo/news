use chrono::Local;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOGGER: OnceLock<Result<Mutex<File>, String>> = OnceLock::new();

pub fn log_path() -> PathBuf {
    let xdg = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/.cache", home)
    });
    PathBuf::from(xdg).join("news").join("news.log")
}

pub fn init() -> Result<(), String> {
    match LOGGER.get_or_init(open_logger) {
        Ok(_) => Ok(()),
        Err(e) => Err(e.clone()),
    }
}

pub fn info(msg: impl AsRef<str>) {
    write_line("INFO", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    write_line("WARN", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    write_line("ERROR", msg.as_ref());
}

fn open_logger() -> Result<Mutex<File>, String> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create log dir: {}", e))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open log file {}: {}", path.display(), e))?;
    Ok(Mutex::new(file))
}

fn write_line(level: &str, msg: &str) {
    let logger = match LOGGER.get_or_init(open_logger) {
        Ok(logger) => logger,
        Err(_) => return,
    };

    if let Ok(mut file) = logger.lock() {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let message = msg.replace('\n', "\\n");
        let _ = writeln!(file, "{} [{}] {}", timestamp, level, message);
    }
}
