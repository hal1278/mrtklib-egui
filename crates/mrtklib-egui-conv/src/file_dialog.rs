//! Native file dialogs run on workers; the UI only polls a result channel.

use crate::{logging, naming};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    Input,
    Directory,
    Output(usize),
}

pub(crate) struct Selection {
    pub target: Target,
    pub path: Option<PathBuf>,
}
pub(crate) struct FileDialog {
    result: Receiver<Result<Selection, String>>,
}

impl FileDialog {
    pub fn open(
        target: Target,
        value: Option<PathBuf>,
        notify: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        Self::spawn(
            move || {
                logging::begin_file_dialog();
                let mut dialog = rfd::FileDialog::new();
                if let Some(value) = value {
                    if matches!(target, Target::Directory) {
                        dialog = dialog.set_directory(&value);
                    } else {
                        if let Some(parent) = value.parent().filter(|p| !p.as_os_str().is_empty()) {
                            dialog = dialog.set_directory(parent);
                        }
                        if let Some(name) = value.file_name() {
                            dialog = dialog.set_file_name(name.to_string_lossy());
                        }
                    }
                }
                let path = match target {
                    Target::Input => dialog.set_title("Input file").pick_file(),
                    Target::Directory => dialog.set_title("Output Directory").pick_folder(),
                    Target::Output(_) => dialog.set_title("Output file").save_file(),
                };
                complete(target, path, logging::take_file_dialog_warning())
            },
            notify,
        )
    }

    fn spawn(
        select: impl FnOnce() -> Result<Selection, String> + Send + 'static,
        notify: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, result) = mpsc::channel();
        std::thread::Builder::new()
            .name("conv-file-dialog".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(select))
                    .unwrap_or_else(|_| Err("file dialog worker panicked".into()));
                let _ = sender.send(result);
                notify();
            })
            .map_err(|e| format!("cannot start file dialog worker: {e}"))?;
        Ok(Self { result })
    }

    pub fn poll(&self) -> Result<Option<Selection>, String> {
        match self.result.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                Err("file dialog worker ended without a result".into())
            }
        }
    }
}

fn complete(
    target: Target,
    path: Option<PathBuf>,
    warning: Option<String>,
) -> Result<Selection, String> {
    if path.is_none()
        && let Some(warning) = warning
    {
        return Err(format!("File dialog unavailable: {warning}"));
    }
    Ok(Selection { target, path })
}

/// Dialog hints must be absolute too. An empty hint uses the OS dialog's default;
/// browsing can replace an invalid relative input without trying to resolve it.
pub(crate) fn hint(
    target: Target,
    value: &str,
    input: &str,
    home: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let expanded = naming::expand_home(value, home)?;
    if expanded.is_absolute() {
        return naming::normalize_absolute(&expanded).map(Some);
    }
    if matches!(target, Target::Input) {
        return Ok(None);
    }
    let input = naming::input_path(input, home)?;
    naming::output_path(value, &input, home).map(Some)
}

pub(crate) fn selected_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Selected path cannot be represented as UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn polling_does_not_wait_for_dialog_and_completion_requests_repaint() {
        let (release, wait) = mpsc::channel();
        let (repaint, wake) = mpsc::channel();
        let dialog = FileDialog::spawn(
            move || {
                wait.recv().unwrap();
                Ok(Selection {
                    target: Target::Input,
                    path: Some("input.ubx".into()),
                })
            },
            move || repaint.send(()).unwrap(),
        )
        .unwrap();
        assert!(dialog.poll().unwrap().is_none());
        release.send(()).unwrap();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            dialog.poll().unwrap().unwrap().path.unwrap(),
            Path::new("input.ubx")
        );
    }
    #[test]
    fn cancellation_is_delivered_without_a_path() {
        let (repaint, wake) = mpsc::channel();
        let dialog = FileDialog::spawn(
            || {
                Ok(Selection {
                    target: Target::Output(0),
                    path: None,
                })
            },
            move || repaint.send(()).unwrap(),
        )
        .unwrap();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(dialog.poll().unwrap().unwrap().path.is_none());
    }

    #[test]
    fn failed_dialog_is_reported_through_the_worker_channel_and_message_area() {
        let (repaint, wake) = mpsc::channel();
        let dialog = FileDialog::spawn(
            || complete(Target::Input, None, Some("libdbus-1.so not found".into())),
            move || repaint.send(()).unwrap(),
        )
        .unwrap();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        let error = dialog.poll().err().unwrap();
        assert_eq!(error, "File dialog unavailable: libdbus-1.so not found");
        assert_eq!(crate::view::build(None, Some(&error)).message, error);
        assert!(complete(Target::Input, None, None).unwrap().path.is_none());
        assert_eq!(
            complete(
                Target::Input,
                Some("/selected.ubx".into()),
                Some("fallback warning".into())
            )
            .unwrap()
            .path,
            Some("/selected.ubx".into())
        );
    }

    #[test]
    fn dialog_hints_use_the_input_directory_or_home_and_never_a_relative_directory() {
        let home = std::env::temp_dir().join("conv-dialog-home");
        let input = home.join("data/in.ubx");
        let input = input.to_str().unwrap();
        for target in [Target::Directory, Target::Output(0)] {
            assert_eq!(
                hint(target, "../out", input, Some(&home)).unwrap(),
                Some(home.join("out"))
            );
            assert_eq!(
                hint(target, "~/out", input, Some(&home)).unwrap(),
                Some(home.join("out"))
            );
            assert!(
                hint(target, "~/out", input, None)
                    .unwrap_err()
                    .contains("home directory is unavailable")
            );
        }
        assert_eq!(
            hint(Target::Input, "~/in.ubx", "", Some(&home)).unwrap(),
            Some(home.join("in.ubx"))
        );
        assert!(
            hint(Target::Input, "relative.ubx", "", None)
                .unwrap()
                .is_none()
        );
        assert!(hint(Target::Directory, "", "", None).unwrap().is_none());
    }
}
