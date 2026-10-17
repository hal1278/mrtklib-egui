//! Native file dialogs run on workers; the UI only polls a result channel.

use crate::logging;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Copy, Debug)]
pub enum Target {
    Input,
    Directory,
    Output(usize),
}

pub struct Selection {
    pub target: Target,
    pub path: Option<PathBuf>,
}
pub struct FileDialog {
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
            .name("file-dialog".into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, time::Duration};
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
}
