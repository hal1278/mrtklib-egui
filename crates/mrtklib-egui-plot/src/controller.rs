//! Own asynchronous file loading, plot data and GUI settings; no egui dependency.
use crate::{
    data,
    reader::{self, Dataset},
    settings::{self, Options, Settings},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
};

type Loaded = (PathBuf, Result<Dataset, String>);
pub(crate) struct Controller {
    pub settings: Settings,
    pub dataset: Option<Dataset>,
    pub samples: Vec<data::Sample>,
    pub error: Option<String>,
    pub loading: bool,
    pub revision: u64,
    settings_path: Result<PathBuf, String>,
    settings_readable: bool,
    pending: Option<Receiver<Loaded>>,
    /// Clear during a load keeps the worker owned (one reader at a time) and drops its result.
    discard_pending: bool,
    notify: Arc<dyn Fn() + Send + Sync>,
}
impl Controller {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let path = std::env::current_exe()
            .map_err(|e| format!("cannot locate GUI: {e}"))
            .and_then(|p| settings::settings_file(&p));
        let loaded = path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|p| settings::load(p));
        let readable = loaded.is_ok();
        let (settings, error) = match loaded {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!(
                    "{e}; using defaults; settings will not be overwritten"
                )),
            ),
        };
        let mut controller = Self {
            settings,
            dataset: None,
            samples: vec![],
            error,
            loading: false,
            revision: 0,
            settings_path: path,
            settings_readable: readable,
            pending: None,
            discard_pending: false,
            notify: Arc::new(notify),
        };
        if let Some(path) = controller.settings.last_file.clone() {
            controller.open(path);
        }
        controller
    }
    pub fn open(&mut self, path: PathBuf) {
        if self.loading {
            self.error =
                Some("A solution is already loading; wait before opening another file".into());
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let notify = self.notify.clone();
        match std::thread::Builder::new()
            .name("plot-solution-reader".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| reader::read(&path))
                    .unwrap_or_else(|_| Err("solution reader worker panicked".into()));
                let _ = sender.send((path, result));
                notify();
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.loading = true;
                self.discard_pending = false;
            }
            Err(e) => self.error = Some(format!("cannot start solution reader: {e}")),
        }
    }
    pub fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.loading = false;
                if !std::mem::take(&mut self.discard_pending) {
                    self.error = Some("solution reader ended without a result".into());
                }
                return;
            }
        };
        self.pending = None;
        self.loading = false;
        if std::mem::take(&mut self.discard_pending) {
            return;
        }
        match result {
            (path, Ok(dataset)) => {
                self.dataset = Some(dataset);
                self.settings.last_file = Some(path);
                self.error = None;
                self.rebuild();
                self.persist();
            }
            (_, Err(error)) => self.error = Some(error),
        }
    }
    pub fn rebuild(&mut self) {
        self.revision += 1;
        self.samples.clear();
        if let Some(data) = &self.dataset {
            match data::samples(data, &self.settings.options, self.settings.quality) {
                Ok(samples) => self.samples = samples,
                Err(e) => self.error = Some(e),
            }
        }
    }
    pub fn apply_options(&mut self, options: Options) -> Result<(), String> {
        options.validate()?;
        if let Some(dataset) = &self.dataset {
            data::samples(dataset, &options, self.settings.quality)?;
        }
        self.settings.options = options;
        self.error = None;
        self.rebuild();
        self.persist();
        Ok(())
    }
    pub fn clear(&mut self) {
        // A running reader stays owned until it finishes, so Open stays refused and
        // Open/Clear cannot start unbounded readers; its result is dropped in poll.
        self.discard_pending = self.loading;
        self.dataset = None;
        self.samples.clear();
        self.settings.last_file = None;
        self.error = None;
        self.revision += 1;
        self.persist();
    }
    pub fn persist(&mut self) {
        if !self.settings_readable {
            return;
        }
        if let Err(e) = self
            .settings_path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|p| settings::save(p, &self.settings))
        {
            self.error = Some(e);
        }
    }
    #[cfg(test)]
    pub fn fixture(dataset: Dataset) -> Self {
        let mut c = Self {
            settings: Settings::default(),
            dataset: Some(dataset),
            samples: vec![],
            error: None,
            loading: false,
            revision: 0,
            settings_path: Err("test settings are not saved".into()),
            settings_readable: false,
            pending: None,
            discard_pending: false,
            notify: Arc::new(|| {}),
        };
        c.rebuild();
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn asynchronous_open_failure_retains_data_and_clear_discards_it() {
        let directory = std::env::temp_dir().join(format!(
            "plot-load-{}",
            mrtklib_egui_supervisor::generate_token()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("input.pos");
        std::fs::write(&path,"% GPST latitude(deg) longitude(deg) height(m) Q ns sdn(m) sde(m) sdu(m) sdne(m) sdeu(m) sdun(m) age(s) ratio\n2024/08/22 11:00:00.000 35 139 40 1 15 .1 .2 .3 0 0 0 1 3\n").unwrap();
        let (notify, wake) = mpsc::channel();
        let mut c = Controller::fixture(reader::synthetic());
        c.notify = Arc::new(move || notify.send(()).unwrap());
        c.open(path.clone());
        assert!(c.loading);
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert!(!c.loading);
        assert_eq!(c.dataset.as_ref().unwrap().points.len(), 1);
        assert!(c.error.is_none());
        assert_eq!(c.settings.last_file, Some(path.clone()));
        c.open(directory.join("absent.pos"));
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert_eq!(c.dataset.as_ref().unwrap().points.len(), 1);
        assert_eq!(c.settings.last_file, Some(path));
        assert!(c.error.as_ref().unwrap().contains("cannot open solution"));
        c.clear();
        assert!(c.dataset.is_none());
        assert!(c.samples.is_empty());
        assert!(c.settings.last_file.is_none());
        assert!(c.error.is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn clear_during_load_keeps_one_reader_and_drops_its_result() {
        let (release, gate) = mpsc::channel::<()>();
        let (notify, wake) = mpsc::channel();
        let mut c = Controller::fixture(reader::synthetic());
        c.notify = Arc::new(move || notify.send(()).unwrap());
        // A reader that has not finished: install the pending channel directly.
        let (sender, receiver) = mpsc::channel();
        c.pending = Some(receiver);
        c.loading = true;
        let worker = std::thread::spawn(move || {
            gate.recv().unwrap();
            sender
                .send((PathBuf::from("late.pos"), Ok(reader::synthetic())))
                .unwrap();
        });
        c.clear();
        assert!(c.loading, "the running reader stays owned after Clear");
        assert!(c.dataset.is_none());
        c.open(PathBuf::from("other.pos"));
        assert!(c.error.as_ref().unwrap().contains("already loading"));
        release.send(()).unwrap();
        worker.join().unwrap();
        c.poll();
        assert!(!c.loading);
        assert!(
            c.dataset.is_none(),
            "a cleared load must not repopulate the plot"
        );
        assert!(c.settings.last_file.is_none());
        drop(wake);
    }
}
