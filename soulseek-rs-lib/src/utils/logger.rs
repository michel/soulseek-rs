use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{
    Mutex, Once,
    atomic::{AtomicBool, Ordering},
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
static mut LOG_LEVEL: LogLevel = LogLevel::Warn;

static BUFFER: Mutex<Vec<String>> = Mutex::new(Vec::new());
static BUFFERING: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn init() {
    INIT.call_once(|| {
        let level = env::var("LOG_LEVEL")
            .or_else(|_| env::var("RUST_LOG"))
            .unwrap_or_else(|_| "WARN".to_string())
            .to_uppercase();

        unsafe {
            LOG_LEVEL = match level.as_str() {
                "ERROR" => LogLevel::Error,
                "INFO" => LogLevel::Info,
                "DEBUG" | "VERBOSE" => LogLevel::Debug, // Map VERBOSE to DEBUG
                "TRACE" => LogLevel::Trace,
                _ => LogLevel::Warn, // "WARN" or default
            };
        }

        // Initialize log file if LOG_FILE env var is set
        if let Ok(log_file_path) = env::var("LOG_FILE") {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path)
            {
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
    unsafe {
        if level <= LOG_LEVEL {
            let level_str = match level {
                LogLevel::Error => "\x1b[31mERROR\x1b[0m", // Red
                LogLevel::Warn => "\x1b[33mWARN\x1b[0m",   // Yellow
                LogLevel::Info => "\x1b[32mINFO\x1b[0m",   // Green
                LogLevel::Debug => "\x1b[34mDEBUG\x1b[0m", // Blue
                LogLevel::Trace => "\x1b[35mTRACE\x1b[0m", // Magenta
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

            let level_str_plain = match level {
                LogLevel::Error => "ERROR",
                LogLevel::Warn => "WARN",
                LogLevel::Info => "INFO",
                LogLevel::Debug => "DEBUG",
                LogLevel::Trace => "TRACE",
            };

            let formatted_message = format!(
                "[{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}.{subsec_millis:03}] [{level_str}] {message}"
            );

            let formatted_message_plain = format!(
                "[{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}.{subsec_millis:03}] [{level_str_plain}] {message}"
            );

            match choose_sink(BUFFERING.load(Ordering::Relaxed), has_log_file())
            {
                LogSink::File => {
                    if let Ok(mut log_file) = LOG_FILE.lock()
                        && let Some(file) = log_file.as_mut()
                    {
                        let _ = writeln!(file, "{formatted_message_plain}");
                        let _ = file.flush();
                    }
                }
                LogSink::Buffer => {
                    if let Ok(mut buffer) = BUFFER.lock() {
                        buffer.push(formatted_message);
                    }
                }
                LogSink::Stderr => {
                    eprintln!("{formatted_message}");
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

pub fn flush_buffered_logs() {
    disable_buffering();

    if let Ok(mut buffer) = BUFFER.lock() {
        // Write to file if configured, otherwise to stderr
        if let Ok(mut log_file) = LOG_FILE.lock() {
            if let Some(file) = log_file.as_mut() {
                for message in buffer.iter() {
                    // Strip ANSI codes for file output
                    let plain_message = strip_ansi_codes(message);
                    let _ = writeln!(file, "{plain_message}");
                }
                let _ = file.flush();
            } else {
                for message in buffer.iter() {
                    eprintln!("{message}");
                }
            }
        } else {
            // If lock fails, fall back to stderr
            for message in buffer.iter() {
                eprintln!("{message}");
            }
        }
        buffer.clear();
    }
}

fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ANSI escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
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
    use super::{LogSink, choose_sink};

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
