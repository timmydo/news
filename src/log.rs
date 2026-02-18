use std::path::PathBuf;

pub fn log_path() -> PathBuf {
    let xdg = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/.cache", home)
    });
    PathBuf::from(xdg).join("news").join("news.log")
}
