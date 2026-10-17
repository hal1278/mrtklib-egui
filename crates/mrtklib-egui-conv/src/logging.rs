//! Show rfd and conv warnings, retaining only rfd diagnostics for the active file dialog.
//! Other crates' warnings (for example graphics driver probing) are not shown: they are
//! noise for a user and were not printed before this logger existed.

use log::{Level, LevelFilter, Log, Metadata, Record};
use mrtklib_egui_supervisor::mask::mask;
use std::sync::Mutex;

#[derive(Default)]
struct DialogLogger {
    last_rfd: Mutex<Option<String>>,
}

static LOGGER: DialogLogger = DialogLogger {
    last_rfd: Mutex::new(None),
};

impl DialogLogger {
    fn take(&self) -> Option<String> {
        self.last_rfd
            .lock()
            .expect("rfd diagnostic lock poisoned")
            .take()
    }
}

impl Log for DialogLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        shown(metadata.target(), metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = mask(&record.args().to_string(), &[]);
        eprintln!("{} {}: {message}", record.level(), record.target());
        if is_rfd(record.target()) {
            *self.last_rfd.lock().expect("rfd diagnostic lock poisoned") = Some(message);
        }
    }

    fn flush(&self) {}
}

fn is_rfd(target: &str) -> bool {
    target == "rfd" || target.starts_with("rfd::")
}

fn shown(target: &str, level: Level) -> bool {
    let conv = target == "mrtklib_egui_conv" || target.starts_with("mrtklib_egui_conv::");
    level <= Level::Warn && (is_rfd(target) || conv)
}

pub(crate) fn install() -> Result<(), log::SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(LevelFilter::Warn);
    Ok(())
}

/// The UI allows only one pending dialog. Clear stale diagnostics before opening it,
/// and consume its last warning/error after rfd returns (including fallback errors).
pub(crate) fn begin_file_dialog() {
    LOGGER.take();
}

pub(crate) fn take_file_dialog_warning() -> Option<String> {
    LOGGER.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(logger: &DialogLogger, target: &str, level: Level, message: &str) {
        logger.log(
            &Record::builder()
                .target(target)
                .level(level)
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn shows_only_rfd_and_conv_warnings() {
        assert!(shown("rfd::backend::xdg_desktop_portal", Level::Error));
        assert!(shown("mrtklib_egui_conv::file_dialog", Level::Warn));
        assert!(!shown("wgpu_hal::vulkan::instance", Level::Warn));
        assert!(!shown("zbus::proxy", Level::Error));
        assert!(!shown("rfd_other", Level::Warn));
        assert!(!shown("rfd", Level::Info));
    }

    #[test]
    fn captures_last_rfd_warning_or_error_and_masks_before_retaining() {
        let logger = DialogLogger::default();
        emit(
            &logger,
            "rfd::backend::portal",
            Level::Error,
            "libdbus-1.so not found",
        );
        emit(
            &logger,
            "rfd",
            Level::Warn,
            "failed ntrip://user:password@example.com",
        );
        emit(&logger, "rfd::backend", Level::Info, "ignored information");
        emit(&logger, "other", Level::Error, "unrelated error");
        emit(&logger, "rfd_other", Level::Warn, "unrelated warning");
        assert_eq!(
            logger.take().as_deref(),
            Some("failed ntrip://***@example.com")
        );
        assert!(
            logger.take().is_none(),
            "a consumed diagnostic cannot leak to a later dialog"
        );
    }
}
