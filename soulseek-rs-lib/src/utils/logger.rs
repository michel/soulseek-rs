use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex, Once,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

static INIT: Once = Once::new();
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Warn as u8);

/// Change the level at runtime, from any thread.
pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

#[must_use]
pub fn log_level() -> LogLevel {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Error,
        2 => LogLevel::Info,
        3 => LogLevel::Debug,
        4 => LogLevel::Trace,
        _ => LogLevel::Warn,
    }
}

static BUFFER: Mutex<Vec<String>> = Mutex::new(Vec::new());
static BUFFERING: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<LogFile>> = Mutex::new(None);
const MAX_LOG_FILE_SIZE: u64 = 16 * 1024 * 1024;

struct LogFile {
    file: File,
    path: PathBuf,
    size: u64,
}

impl LogFile {
    fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            path: path.to_path_buf(),
            size,
        })
    }

    fn write_line(&mut self, line: &str, limit: u64) {
        if self.size > limit {
            let _ = self.rotate();
        }
        let _ = writeln!(self.file, "{line}");
        // Count attempted bytes so partial write failures cannot hide growth.
        self.size = self
            .size
            .saturating_add(line.len() as u64)
            .saturating_add(1);
        let _ = self.file.flush();
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let mut previous = self.path.as_os_str().to_os_string();
        previous.push(".1");
        // A file that is no longer at its path was moved aside from outside:
        // that is a rotation already, and only the fresh file is missing.
        match std::fs::rename(&self.path, previous) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e);
            }
            _ => {}
        }
        *self = Self::open(&self.path)?;
        Ok(())
    }
}

pub fn init() {
    INIT.call_once(|| {
        let level = env::var("LOG_LEVEL")
            .or_else(|_| env::var("RUST_LOG"))
            .unwrap_or_else(|_| "WARN".to_string())
            .to_uppercase();

        set_log_level(match level.as_str() {
            "ERROR" => LogLevel::Error,
            "INFO" => LogLevel::Info,
            "DEBUG" | "VERBOSE" => LogLevel::Debug,
            "TRACE" => LogLevel::Trace,
            _ => LogLevel::Warn,
        });

        // Initialize log file if LOG_FILE env var is set
        if let Ok(log_file_path) = env::var("LOG_FILE") {
            match LogFile::open(Path::new(&log_file_path)) {
                Ok(file) => {
                    if let Ok(mut log_file) = LOG_FILE.lock() {
                        *log_file = Some(file);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to open log file '{log_file_path}': {e}");
                }
            }
        }
    });
}

/// Where a single log line should be written. Each line goes to exactly one
/// sink; routing a line to more than one is what caused file lines to be
/// duplicated (written once eagerly and again when the buffer was flushed).
#[derive(Debug, PartialEq, Eq)]
enum LogSink {
    File,
    Buffer,
    Stderr,
}

const fn choose_sink(buffering: bool, has_file: bool) -> LogSink {
    match (buffering, has_file) {
        // A configured file always takes the line directly and bypasses
        // buffering, so the flush step never re-writes it.
        (_, true) => LogSink::File,
        // No file: while the TUI holds the screen we defer stderr writes.
        (true, false) => LogSink::Buffer,
        (false, false) => LogSink::Stderr,
    }
}

fn has_log_file() -> bool {
    LOG_FILE.lock().is_ok_and(|f| f.is_some())
}

pub fn log(level: LogLevel, message: &str) {
    if level <= log_level() {
        {
            let (name, colour) = match level {
                LogLevel::Error => ("ERROR", "\x1b[31m"), // Red
                LogLevel::Warn => ("WARN", "\x1b[33m"),   // Yellow
                LogLevel::Info => ("INFO", "\x1b[32m"),   // Green
                LogLevel::Debug => ("DEBUG", "\x1b[34m"), // Blue
                LogLevel::Trace => ("TRACE", "\x1b[35m"), // Magenta
            };

            let now = std::time::SystemTime::now();
            let datetime = now
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = datetime.as_secs();
            let subsec_millis = datetime.subsec_millis();

            // Format as YYYY-MM-DD HH:MM:SS.mmm
            let days_since_epoch = secs / 86400;
            let days_since_1970 = days_since_epoch as i32;

            // Calculate year (approximately)
            let mut year = 1970;
            let mut remaining_days = days_since_1970;

            while remaining_days >= 365 {
                let is_leap =
                    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
                let days_in_year = if is_leap { 366 } else { 365 };
                if remaining_days >= days_in_year {
                    remaining_days -= days_in_year;
                    year += 1;
                } else {
                    break;
                }
            }

            // Calculate month and day (simplified)
            let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let is_leap =
                (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            let mut month = 1;
            let mut day = remaining_days + 1;

            for &days_in_month in &month_days {
                let actual_days = if month == 2 && is_leap {
                    29
                } else {
                    days_in_month
                };
                if day > actual_days {
                    day -= actual_days;
                    month += 1;
                } else {
                    break;
                }
            }

            // Calculate time of day
            let seconds_in_day = secs % 86400;
            let hours = seconds_in_day / 3600;
            let minutes = (seconds_in_day % 3600) / 60;
            let seconds = seconds_in_day % 60;

            let timestamp = format!(
                "{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}.{subsec_millis:03}"
            );

            match choose_sink(BUFFERING.load(Ordering::Relaxed), has_log_file())
            {
                LogSink::File => {
                    if let Ok(mut log_file) = LOG_FILE.lock()
                        && let Some(file) = log_file.as_mut()
                    {
                        file.write_line(
                            &format!("[{timestamp}] [{name}] {message}"),
                            MAX_LOG_FILE_SIZE,
                        );
                    }
                }
                LogSink::Buffer => {
                    if let Ok(mut buffer) = BUFFER.lock() {
                        buffer.push(format!(
                            "[{timestamp}] [{colour}{name}\x1b[0m] {message}"
                        ));
                    }
                }
                LogSink::Stderr => {
                    eprintln!(
                        "[{timestamp}] [{colour}{name}\x1b[0m] {message}"
                    );
                }
            }
        }
    }
}

pub fn enable_buffering() {
    BUFFERING.store(true, Ordering::Relaxed);
}

pub fn disable_buffering() {
    BUFFERING.store(false, Ordering::Relaxed);
}

/// Only lines that were buffered land here, and a line is buffered only when
/// no log file is configured, so the buffer is always stderr-bound.
pub fn flush_buffered_logs() {
    disable_buffering();

    if let Ok(mut buffer) = BUFFER.lock() {
        for message in buffer.iter() {
            eprintln!("{message}");
        }
        buffer.clear();
    }
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::utils::logger::log($crate::utils::logger::LogLevel::Error, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::utils::logger::log($crate::utils::logger::LogLevel::Warn, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::utils::logger::log($crate::utils::logger::LogLevel::Info, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::utils::logger::log($crate::utils::logger::LogLevel::Debug, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::utils::logger::log($crate::utils::logger::LogLevel::Trace, &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::{
        LogFile, LogLevel, LogSink, choose_sink, log_level, set_log_level,
    };

    fn log_directory(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-log-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn a_log_file_keeps_only_two_bounded_generations_and_the_newest_line() {
        let dir = log_directory("rotation");
        let path = dir.join("daemon.log");
        let mut file = LogFile::open(&path).unwrap();
        for n in 0..100 {
            file.write_line(&format!("line {n:03}"), 64);
        }
        drop(file);

        let live = std::fs::read_to_string(&path).unwrap();
        assert!(
            live.len() <= 64 + "line 099\n".len(),
            "the live log must be bounded, got {} bytes",
            live.len()
        );
        assert!(live.ends_with("line 099\n"));
        let previous =
            std::fs::read_to_string(dir.join("daemon.log.1")).unwrap();
        assert!(!previous.is_empty());
        assert!(previous.len() <= 64 + "line 099\n".len());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_log_moved_aside_from_outside_stops_growing_at_the_limit() {
        let dir = log_directory("moved-aside");
        let path = dir.join("daemon.log");
        let mut file = LogFile::open(&path).unwrap();
        file.write_line("before the move", 8);
        std::fs::rename(&path, dir.join("daemon.log.old")).unwrap();
        file.write_line("after the move", 8);
        drop(file);

        assert_eq!(
            std::fs::read_to_string(dir.join("daemon.log.old")).unwrap(),
            "before the move\n",
            "past the limit the moved file is left alone"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after the move\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_rotation_keeps_writing_to_the_current_log() {
        let dir = log_directory("failed-rotation");
        let path = dir.join("daemon.log");
        std::fs::create_dir(dir.join("daemon.log.1")).unwrap();
        std::fs::write(&path, "existing log\n").unwrap();
        let mut file = LogFile::open(&path).unwrap();
        file.write_line("first new line", 8);
        file.write_line("newest line", 8);
        drop(file);

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "existing log\nfirst new line\nnewest line\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_level_can_be_changed_while_running() {
        let before = log_level();
        set_log_level(LogLevel::Trace);
        assert_eq!(log_level(), LogLevel::Trace);
        set_log_level(before);
        assert_eq!(log_level(), before);
    }

    #[test]
    fn a_configured_file_bypasses_buffering_so_lines_are_not_duplicated() {
        // With a file configured, the line goes straight to the file whether or
        // not buffering is on, so it is never also queued for a flush that would
        // write it a second time.
        assert_eq!(choose_sink(true, true), LogSink::File);
        assert_eq!(choose_sink(false, true), LogSink::File);
    }

    #[test]
    fn without_a_file_buffering_routes_to_the_buffer_else_stderr() {
        assert_eq!(choose_sink(true, false), LogSink::Buffer);
        assert_eq!(choose_sink(false, false), LogSink::Stderr);
    }
}
