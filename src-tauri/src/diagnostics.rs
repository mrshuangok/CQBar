use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use chrono::Utc;

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn init() {
    if let Some(base) = dirs::data_local_dir() {
        let path = base.join("codex-quota-float").join("app.log");
        let _ = LOG_PATH.set(path);
    }
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_else(|| "unknown".to_string());
        event("ERROR", "panic", &format!("location={location}"));
    }));
    event(
        "INFO",
        "app.start",
        &format!(
            "pid={} version={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        ),
    );
}

pub(crate) fn event(level: &str, name: &str, detail: &str) {
    let line = format!(
        "{} {level} {name} {}\n",
        Utc::now().to_rfc3339(),
        detail.replace('\r', " ").replace('\n', " ")
    );
    if let Err(error) = append(&line) {
        eprintln!("diagnostic log unavailable: {error}; {line}");
    }
}

fn append(line: &str) -> io::Result<()> {
    let path = LOG_PATH
        .get()
        .ok_or_else(|| io::Error::other("local app data directory not found"))?;
    append_to(path, line)
}

fn append_to(path: &PathBuf, line: &str) -> io::Result<()> {
    let _guard = LOG_LOCK
        .lock()
        .map_err(|_| io::Error::other("log lock poisoned"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() >= MAX_LOG_BYTES) {
        let previous = path.with_extension("log.old");
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, previous)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_is_bounded_and_keeps_previous_segment() {
        let path = std::env::temp_dir().join(format!(
            "codex-quota-log-test-{}-{}.log",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_LOG_BYTES).unwrap();
        append_to(&path, "new event\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new event\n");
        assert_eq!(
            fs::metadata(path.with_extension("log.old")).unwrap().len(),
            MAX_LOG_BYTES
        );
        fs::remove_file(&path).unwrap();
        fs::remove_file(path.with_extension("log.old")).unwrap();
    }
}
