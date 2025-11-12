pub fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    format!("{:.1} MB", mb)
}

pub fn format_bytes_progress(downloaded: u64, total: u64) -> String {
    let downloaded_mb = downloaded as f64 / 1_048_576.0;
    let total_mb = total as f64 / 1_048_576.0;
    format!("{:.1}/{:.1} MB", downloaded_mb, total_mb)
}

pub fn format_speed(speed_bytes_per_sec: f64) -> String {
    let mb = speed_bytes_per_sec / 1_048_576.0;
    format!("{:.1} MB/s", mb)
}

pub fn get_bitrate(attribs: &std::collections::HashMap<u32, u32>) -> Option<u32> {
    attribs.get(&0).copied()
}
