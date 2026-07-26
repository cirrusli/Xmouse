use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn format_uptime(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3_600 {
        format!("{} 小时 {} 分", seconds / 3_600, seconds % 3_600 / 60)
    } else if seconds >= 60 {
        format!("{} 分 {} 秒", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} 秒")
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    if bytes >= MIB as u64 {
        format!("{:.1} MiB", bytes as f64 / MIB)
    } else if bytes >= KIB as u64 {
        format!("{:.0} KiB", bytes as f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn relative_history_time(timestamp: i64) -> String {
    let timestamp = if timestamp > 10_000_000_000 {
        timestamp / 1_000
    } else {
        timestamp
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(timestamp);
    let seconds = now.saturating_sub(timestamp).max(0);
    match seconds {
        0..=59 => "刚刚".to_owned(),
        60..=3_599 => format!("{} 分钟前", seconds / 60),
        3_600..=86_399 => format!("{} 小时前", seconds / 3_600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_compact_resource_values() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2_048), "2 KiB");
        assert_eq!(format_bytes(1_572_864), "1.5 MiB");
        assert_eq!(format_uptime(Duration::from_secs(125)), "2 分 5 秒");
    }
}
