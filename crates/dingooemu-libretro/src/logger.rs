use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::callbacks;
use crate::constants::{RETRO_LOG_DEBUG, RETRO_LOG_ERROR, RETRO_LOG_INFO, RETRO_LOG_WARN};

struct LibretroLogger;

static LOGGER: LibretroLogger = LibretroLogger;
static DEBUG_LOGGING: AtomicBool = AtomicBool::new(false);

impl log::Log for LibretroLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info || DEBUG_LOGGING.load(Ordering::Relaxed)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = match record.level() {
            log::Level::Error => RETRO_LOG_ERROR,
            log::Level::Warn => RETRO_LOG_WARN,
            log::Level::Info => RETRO_LOG_INFO,
            log::Level::Debug | log::Level::Trace => RETRO_LOG_DEBUG,
        };
        let text = format!("[DingooEmu] {}\n", record.args()).replace('%', "%%");
        if let Ok(message) = CString::new(text) {
            callbacks::log(level, message.as_ptr());
        }
    }

    fn flush(&self) {}
}

pub fn initialize() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

pub fn set_debug_logging(enabled: bool) {
    DEBUG_LOGGING.store(enabled, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    #[test]
    fn debug_records_follow_live_setting() {
        let debug = log::Metadata::builder().level(log::Level::Debug).build();
        let info = log::Metadata::builder().level(log::Level::Info).build();
        set_debug_logging(false);
        assert!(!LOGGER.enabled(&debug));
        assert!(LOGGER.enabled(&info));
        set_debug_logging(true);
        assert!(LOGGER.enabled(&debug));
        set_debug_logging(false);
    }
}
