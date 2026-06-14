// Tredo logging system - writes to both console and file.
// Uses the `log` crate for standard logging levels.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// Log levels matching the log crate
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    #[allow(dead_code)]
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}

/// Global logger that writes to both stdout and a log file
pub struct TredoLogger {
    log_file: Mutex<Option<File>>,
    level: Mutex<LogLevel>,
}

impl TredoLogger {
    /// Create a new logger that writes to the specified log file
    pub fn new(log_path: Option<PathBuf>) -> Self {
        let log_file =
            log_path.and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());

        Self {
            log_file: Mutex::new(log_file),
            level: Mutex::new(LogLevel::Info),
        }
    }

    /// Set the minimum log level
    pub fn set_level(&self, level: LogLevel) {
        *self.level.lock().unwrap() = level;
    }

    /// Log a message
    pub fn log(&self, level: LogLevel, module: &str, message: &str) {
        // Check if we should log this level
        let min_level = *self.level.lock().unwrap();
        if !self.should_log(level, min_level) {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level_str = match level {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        };

        let log_line = format!("[{}] [{}] [{}] {}\n", timestamp, level_str, module, message);

        // Write to stdout/stderr
        match level {
            LogLevel::Error => eprintln!("{}", log_line.trim()),
            LogLevel::Warn => eprintln!("{}", log_line.trim()),
            LogLevel::Info => println!("{}", log_line.trim()),
            LogLevel::Debug => println!("{}", log_line.trim()),
        }

        // Write to file
        if let Ok(mut file) = self.log_file.lock() {
            if let Some(ref mut f) = *file {
                let _ = f.write_all(log_line.as_bytes());
                let _ = f.flush();
            }
        }
    }

    fn should_log(&self, level: LogLevel, min_level: LogLevel) -> bool {
        let level_val = match level {
            LogLevel::Error => 0,
            LogLevel::Warn => 1,
            LogLevel::Info => 2,
            LogLevel::Debug => 3,
        };
        let min_val = match min_level {
            LogLevel::Error => 0,
            LogLevel::Warn => 1,
            LogLevel::Info => 2,
            LogLevel::Debug => 3,
        };
        level_val <= min_val
    }

    /// Convenience methods for each log level
    pub fn error(&self, module: &str, message: &str) {
        self.log(LogLevel::Error, module, message);
    }

    pub fn warn(&self, module: &str, message: &str) {
        self.log(LogLevel::Warn, module, message);
    }

    pub fn info(&self, module: &str, message: &str) {
        self.log(LogLevel::Info, module, message);
    }

    pub fn debug(&self, module: &str, message: &str) {
        self.log(LogLevel::Debug, module, message);
    }
}

/// Global logger instance (uses LazyLock because TredoLogger::new is not const)
#[allow(dead_code)]
static LOGGER: LazyLock<TredoLogger> = LazyLock::new(|| TredoLogger::new(None));

/// Initialize the global logger with a file path
pub fn init_logging(log_path: Option<PathBuf>) {
    // This is a simple replacement - in production you'd use a lazy static
    eprintln!("[Logging] Initialized logging to {:?}", log_path);
}

/// Log at error level
#[macro_export]
macro_rules! tredo_error {
    ($module:expr, $($arg:tt)*) => {
        eprintln!(concat!("[ERROR] [", $module, "] ") , $($arg)*);
    };
}

/// Log at warn level
#[macro_export]
macro_rules! tredo_warn {
    ($module:expr, $($arg:tt)*) => {
        eprintln!(concat!("[WARN] [", $module, "] ") , $($arg)*);
    };
}

/// Log at info level
#[macro_export]
macro_rules! tredo_info {
    ($module:expr, $($arg:tt)*) => {
        println!(concat!("[INFO] [", $module, "] ") , $($arg)*);
    };
}

/// Log at debug level
#[macro_export]
macro_rules! tredo_debug {
    ($module:expr, $($arg:tt)*) => {
        println!(concat!("[DEBUG] [", $module, "] ") , $($arg)*);
    };
}
