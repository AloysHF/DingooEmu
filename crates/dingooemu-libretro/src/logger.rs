use std::ffi::CString;

use crate::callbacks;
use crate::constants::{RETRO_LOG_DEBUG, RETRO_LOG_ERROR, RETRO_LOG_INFO, RETRO_LOG_WARN};

struct LibretroLogger;

static LOGGER: LibretroLogger = LibretroLogger;

impl log::Log for LibretroLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
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
        log::set_max_level(log::LevelFilter::Info);
    }
}
