//! Native file dialogs run on workers; the UI only polls a result channel.

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
        value: &str,
        notify: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        let value = PathBuf::from(value);
        Self::spawn(
            move || {
                let mut dialog = rfd::FileDialog::new();
                if !value.as_os_str().is_empty() {
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
                Ok(Selection { target, path })
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
}
