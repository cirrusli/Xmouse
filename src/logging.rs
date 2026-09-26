use anyhow::{Context, Result};
use std::{
    fmt::Display,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 512 * 1024;
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn init(root: &Path) -> Result<()> {
    fs::create_dir_all(root).with_context(|| format!("创建日志目录失败：{}", root.display()))?;
    let path = root.join("xmouse.log");
    rotate_if_needed(&path)?;
    let _ = LOG_PATH.set(path);
    Ok(())
}

pub fn error(context: &str, error: impl Display) {
    write_line("ERROR", context, error);
}

pub fn info(context: &str, message: impl Display) {
    write_line("INFO", context, message);
}

fn write_line(level: &str, context: &str, message: impl Display) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let Ok(_guard) = LOG_LOCK.lock() else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let safe_context = context.replace(['\r', '\n'], " ");
    let safe_message = message.to_string().replace(['\r', '\n'], " ");
    let _ = rotate_if_needed(path);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{timestamp} {level} {safe_context}: {safe_message}");
    }
}

fn rotate_if_needed(path: &Path) -> Result<()> {
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES) {
        let previous = path.with_file_name("xmouse.log.1");
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, &previous).with_context(|| format!("轮转日志失败：{}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_LOG_BYTES, rotate_if_needed};
    use std::{fs, time::SystemTime};

    #[test]
    fn active_log_rotates_after_reaching_size_limit() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("xmouse-log-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("xmouse.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        rotate_if_needed(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::metadata(directory.join("xmouse.log.1")).unwrap().len(),
            MAX_LOG_BYTES
        );
        fs::remove_file(directory.join("xmouse.log.1")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
