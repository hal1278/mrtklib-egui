//! Own asynchronous file loading, plot data and GUI settings; no egui dependency.
use crate::{
    data,
    reader::{self, Dataset},
    settings::{self, Options, Settings},
    summary,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Selection {
    pub data: [bool; 2],
    pub difference: bool,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            data: [true, false],
            difference: false,
        }
    }
}
impl Selection {
    pub fn primary(self) -> usize {
        usize::from(!self.data[0] && self.data[1])
    }
    pub fn toggle(&mut self, button: usize) {
        if button == 2 {
            self.difference = !self.difference;
            self.data = [false; 2];
        } else {
            self.data[button] = !self.data[button];
            self.difference = false;
        }
    }
}
type Loaded = Vec<(usize, PathBuf, Result<Dataset, String>)>;
pub(crate) struct Controller {
    pub settings: Settings,
    pub dataset: Option<Dataset>,
    pub dataset2: Option<Dataset>,
    pub selection: Selection,
    pub all_samples: [Vec<data::Sample>; 2],
    pub samples: Vec<data::Sample>,
    /// Qualities of the shown samples before the quality filter (difference: matched Q).
    pub present_qualities: [bool; 8],
    pub error: Option<String>,
    pub file_errors: [Vec<String>; 2],
    pub conversion_errors: [Option<String>; 2],
    pub loading: bool,
    pub revision: u64,
    /// Successful file loads are distinct from sample-only revisions.
    pub load_revision: u64,
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
            dataset2: None,
            selection: Selection::default(),
            all_samples: Default::default(),
            samples: vec![],
            present_qualities: [false; 8],
            error,
            file_errors: Default::default(),
            conversion_errors: Default::default(),
            loading: false,
            revision: 0,
            load_revision: 0,
            settings_path: path,
            settings_readable: readable,
            pending: None,
            discard_pending: false,
            notify: Arc::new(notify),
        };
        controller.reload();
        controller
    }
    pub fn dataset_at(&self, slot: usize) -> Option<&Dataset> {
        if slot == 0 {
            self.dataset.as_ref()
        } else {
            self.dataset2.as_ref()
        }
    }
    pub fn last_file(&self, slot: usize) -> Option<PathBuf> {
        if slot == 0 {
            self.settings.last_file.clone()
        } else {
            self.settings.last_file2.clone()
        }
    }
    pub fn override_hz(&self, slot: usize) -> Option<f64> {
        let path = self.last_file(slot)?;
        self.settings
            .manual_rates
            .iter()
            .find(|rate| rate.path == path)
            .map(|rate| rate.hz)
    }
    pub fn rate(&self, slot: usize) -> Option<f64> {
        let data = self.dataset_at(slot)?;
        if data.points.len() < 2 {
            return None;
        }
        self.override_hz(slot)
            .or_else(|| summary::rate(data.points.iter().map(|p| p.time_ns)))
    }
    pub fn rate_text(&self, slot: usize) -> String {
        let Some(hz) = self.rate(slot) else {
            return "—".into();
        };
        let path = self.last_file(slot);
        match self
            .settings
            .manual_rates
            .iter()
            .find(|rate| Some(&rate.path) == path.as_ref())
        {
            Some(rate) => format!(
                "{} Hz (manual)",
                rate.entered.clone().unwrap_or_else(|| hz.to_string())
            ),
            None => format!("{} Hz", summary::rate_text(hz)),
        }
    }
    pub fn set_override_hz(&mut self, slot: usize, entered: Option<&str>) -> Result<(), String> {
        let hz = entered
            .map(|text| {
                text.parse::<f64>()
                    .ok()
                    .filter(|hz| hz.is_finite() && *hz > 0.0)
                    .ok_or("Rate must be positive and finite")
            })
            .transpose()?;
        let path = self
            .last_file(slot)
            .ok_or("Open a file before setting its rate")?;
        self.settings.manual_rates.retain(|rate| rate.path != path);
        if let Some(hz) = hz {
            self.settings.manual_rates.push(settings::ManualRate {
                path,
                hz,
                entered: entered.map(str::to_owned),
            });
        }
        self.persist();
        Ok(())
    }
    pub fn open(&mut self, slot: usize, path: PathBuf) {
        self.start_read(vec![(slot, path)]);
    }
    pub fn reload(&mut self) {
        let paths = (0..2)
            .filter_map(|slot| self.last_file(slot).map(|p| (slot, p)))
            .collect();
        self.start_read(paths);
    }
    fn start_read(&mut self, paths: Vec<(usize, PathBuf)>) {
        if paths.is_empty() {
            return;
        }
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
                let results = paths
                    .into_iter()
                    .map(|(slot, path)| {
                        let result = std::panic::catch_unwind(|| reader::read(&path))
                            .unwrap_or_else(|_| Err("solution reader worker panicked".into()));
                        (slot, path, result)
                    })
                    .collect();
                let _ = sender.send(results);
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
        let mut loaded = false;
        for (slot, path, result) in result {
            match result {
                Ok(dataset) => {
                    self.file_errors[slot].clear();
                    if slot == 0 {
                        self.dataset = Some(dataset);
                        self.settings.last_file = Some(path);
                    } else {
                        self.dataset2 = Some(dataset);
                        self.settings.last_file2 = Some(path);
                    }
                    self.selection.data[slot] = true;
                    self.selection.difference = false;
                    loaded = true;
                }
                Err(error) => {
                    if !self.file_errors[slot].contains(&error) {
                        self.file_errors[slot].push(error);
                    }
                }
            }
        }
        self.error = None;
        if loaded {
            self.load_revision += 1;
            self.rebuild();
            self.persist();
        }
    }

    pub fn toggle(&mut self, button: usize) {
        if button == 2 && !self.difference_available() {
            return;
        }
        let mut next = self.selection;
        next.toggle(button);
        self.selection = next;
        self.error = None;
        self.rebuild();
    }
    pub fn difference_available(&self) -> bool {
        self.settings.plot <= 1
            && (0..2).all(|i| self.dataset_at(i).is_some_and(|d| !d.points.is_empty()))
    }
    pub fn origin(&self, options: &Options) -> Result<data::Origin, String> {
        options.validate()?;
        if options.origin == 5 {
            return Ok(data::Origin::fixed(options.llh));
        }
        let dataset = self
            .dataset
            .as_ref()
            .filter(|d| !d.relative)
            .ok_or("An absolute Solution-1 is required for this coordinate origin")?;
        data::origin(dataset, options)
    }
    fn converted(&self, options: &Options) -> [Result<Vec<data::Sample>, String>; 2] {
        std::array::from_fn(|slot| {
            let Some(dataset) = self.dataset_at(slot) else {
                return Ok(vec![]);
            };
            let origin = if dataset.relative || self.settings.plot == 2 {
                None
            } else {
                Some(self.origin(options)?)
            };
            let mut samples = data::samples_with_origin(dataset, origin.as_ref(), 0);
            for sample in &mut samples {
                sample.source = slot;
            }
            Ok(samples)
        })
    }
    pub fn rebuild(&mut self) {
        self.revision += 1;
        self.samples.clear();
        self.all_samples = Default::default();
        self.conversion_errors = Default::default();
        for (slot, result) in self
            .converted(&self.settings.options)
            .into_iter()
            .enumerate()
        {
            match result {
                Ok(samples) => self.all_samples[slot] = samples,
                Err(error) => self.conversion_errors[slot] = Some(error),
            }
        }
        if self.selection.difference {
            if self.settings.plot != 2 {
                self.samples = data::difference(&self.all_samples[0], &self.all_samples[1], 0);
            }
        } else {
            self.samples = self
                .all_samples
                .iter()
                .enumerate()
                .filter(|(slot, _)| self.selection.data[*slot])
                .flat_map(|(_, s)| s)
                .cloned()
                .collect();
        }
        self.present_qualities = [false; 8];
        for s in &self.samples {
            if let Some(present) = self.present_qualities.get_mut(usize::from(s.quality)) {
                *present = true;
            }
        }
        self.samples
            .retain(|s| self.settings.qualities.visible(s.quality));
        // Plot coordinates are display times, so UTC ticks fall on round UTC times.
        for s in &mut self.samples {
            s.set_vertical(self.settings.height);
            s.time = display_time(s.time, &self.settings.options);
        }
    }
    pub fn apply_options(&mut self, options: Options) -> Result<(), String> {
        options.validate()?;
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
        self.dataset2 = None;
        self.samples.clear();
        self.present_qualities = [false; 8];
        self.all_samples = Default::default();
        self.file_errors = Default::default();
        self.conversion_errors = Default::default();
        self.settings.last_file = None;
        self.settings.last_file2 = None;
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
            dataset2: None,
            selection: Selection::default(),
            all_samples: Default::default(),
            samples: vec![],
            present_qualities: [false; 8],
            error: None,
            file_errors: Default::default(),
            conversion_errors: Default::default(),
            loading: false,
            revision: 0,
            load_revision: 1,
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

/// Display time for the selected format: GPST, or UTC through the leap-second table.
pub(crate) fn display_time(gpst: f64, options: &crate::settings::Options) -> f64 {
    if options.utc && !options.week_time {
        mrtklib_egui_plotkit::leap::gpst_to_utc(gpst)
    } else {
        gpst
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn independent_data_toggles_allow_overlay_none_and_exclude_difference() {
        let mut s = Selection::default();
        for (button, data, difference) in [
            (1, [true, true], false),
            (0, [false, true], false),
            (1, [false, false], false),
            (2, [false, false], true),
            (2, [false, false], false),
            (2, [false, false], true),
            (1, [false, true], false),
            (0, [true, true], false),
        ] {
            s.toggle(button);
            assert_eq!((s.data, s.difference), (data, difference));
        }
    }
    #[test]
    fn second_solution_uses_solution_one_origin_for_every_choice_even_when_hidden() {
        let mut c = Controller::fixture(reader::synthetic());
        c.dataset.as_mut().unwrap().base = Some(c.dataset.as_ref().unwrap().points[0].position);
        let mut second = reader::synthetic();
        for p in &mut second.points {
            p.position[0] += 100.0;
        }
        second.base = Some([6_378_137.0, 0.0, 0.0]);
        c.dataset2 = Some(second);
        c.selection.data = [false, true];
        for origin in 0..6 {
            c.settings.options.origin = origin;
            c.rebuild();
            assert!(c.error.is_none(), "{:?}", c.error);
            let expected = data::origin(c.dataset.as_ref().unwrap(), &c.settings.options).unwrap();
            for (p, s) in c.dataset2.as_ref().unwrap().points.iter().zip(&c.samples) {
                let enu = mrtklib_egui_plotkit::ecef_to_enu(p.position, expected.at(p.time));
                assert_eq!(s.enu, enu);
                assert_eq!(s.source, 1);
            }
        }
    }
    #[test]
    fn an_origin_failure_only_removes_the_dataset_that_needs_it() {
        for plot in [0, 1] {
            for failing_slot in 0..2 {
                let mut c = Controller::fixture(reader::synthetic());
                c.dataset2 = Some(reader::synthetic());
                let relative_slot = 1 - failing_slot;
                let relative = if relative_slot == 0 {
                    c.dataset.as_mut().unwrap()
                } else {
                    c.dataset2.as_mut().unwrap()
                };
                relative.relative = true;
                for (i, p) in relative.points.iter_mut().enumerate() {
                    p.position = [i as f64 * 0.02, 1.0, 0.0];
                }
                c.settings.plot = plot;
                // Average Pos cannot derive an origin from relative Solution-1;
                // Base Station fails for absolute Solution-1 without a base.
                c.settings.options.origin = if failing_slot == 1 { 2 } else { 4 };
                c.dataset.as_mut().unwrap().base = None;
                c.selection.data = std::array::from_fn(|slot| slot == relative_slot);
                c.rebuild();
                let count = c.dataset_at(relative_slot).unwrap().points.len();
                assert_eq!(c.samples.len(), count);
                assert!(c.samples.iter().all(|s| s.source == relative_slot));
                assert!(c.all_samples[failing_slot].is_empty());
                assert!(c.conversion_errors[failing_slot].is_some());
                assert!(c.conversion_errors[relative_slot].is_none());
                c.toggle(failing_slot);
                assert_eq!(
                    c.samples.len(),
                    count,
                    "overlay must retain the valid dataset"
                );
                // A valid user origin recovers the failed slot and clears its error.
                let options = Options {
                    origin: 5,
                    llh: [35.0, 139.0, 40.0],
                    ..Default::default()
                };
                c.apply_options(options).unwrap();
                assert!(c.conversion_errors.iter().all(Option::is_none));
                assert_eq!(c.samples.len(), count * 2);
                c.clear();
                assert!(c.conversion_errors.iter().all(Option::is_none));
            }
        }
    }
    #[test]
    fn applying_an_origin_unavailable_to_one_dataset_keeps_the_other_visible() {
        let mut c = Controller::fixture(reader::synthetic());
        c.dataset.as_mut().unwrap().relative = true;
        c.dataset2 = Some(reader::synthetic());
        let options = Options {
            origin: 2,
            utc: true,
            ..Default::default()
        };
        c.apply_options(options.clone()).unwrap();
        assert_eq!(c.settings.options, options);
        assert_eq!(c.samples.len(), c.dataset.as_ref().unwrap().points.len());
        assert!(c.conversion_errors[1].is_some());
        let invalid = Options {
            y_range: 0.0,
            ..options.clone()
        };
        assert!(c.apply_options(invalid).is_err());
        assert_eq!(c.settings.options, options);
    }
    #[test]
    fn manual_rate_display_preserves_the_entered_precision() {
        let mut c = Controller::fixture(reader::synthetic());
        c.settings.last_file = Some("rate.pos".into());
        assert_eq!(c.rate_text(0), "1.00 Hz");
        c.set_override_hz(0, Some("20.0001")).unwrap();
        assert_eq!(c.rate_text(0), "20.0001 Hz (manual)");
        assert_eq!(c.rate(0), Some(20.0001));
        c.set_override_hz(0, Some("1.0000")).unwrap();
        assert_eq!(c.rate_text(0), "1.0000 Hz (manual)");
    }
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
        c.open(0, path.clone());
        assert!(c.loading);
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert!(!c.loading);
        assert_eq!(c.dataset.as_ref().unwrap().points.len(), 1);
        assert!(c.error.is_none());
        assert_eq!(c.settings.last_file, Some(path.clone()));
        let second_path = directory.join("second.pos");
        std::fs::copy(&path, &second_path).unwrap();
        c.open(1, second_path.clone());
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert_eq!(c.settings.last_file2, Some(second_path.clone()));
        assert_eq!(c.selection.data, [true, true]);
        assert_eq!(c.dataset2.as_ref().unwrap().points.len(), 1);
        // Reload reads both inputs, even when only Data 2 was selected.
        c.toggle(0);
        c.dataset.as_mut().unwrap().points.clear();
        c.dataset2.as_mut().unwrap().points.clear();
        c.reload();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert_eq!(c.dataset.as_ref().unwrap().points.len(), 1);
        assert_eq!(c.dataset2.as_ref().unwrap().points.len(), 1);
        assert_eq!(c.selection.data, [true, true]);
        c.open(0, directory.join("absent.pos"));
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        c.poll();
        assert_eq!(c.dataset.as_ref().unwrap().points.len(), 1);
        assert_eq!(c.settings.last_file, Some(path));
        assert!(
            c.file_errors[0]
                .last()
                .unwrap()
                .contains("cannot open solution")
        );
        c.clear();
        assert!(c.dataset.is_none());
        assert!(c.dataset2.is_none());
        assert!(c.samples.is_empty());
        assert!(c.settings.last_file.is_none());
        assert!(c.settings.last_file2.is_none());
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
                .send(vec![(
                    1,
                    PathBuf::from("late.pos"),
                    Ok(reader::synthetic()),
                )])
                .unwrap();
        });
        c.clear();
        assert!(c.loading, "the running reader stays owned after Clear");
        assert!(c.dataset.is_none());
        c.open(1, PathBuf::from("other.pos"));
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
        assert!(c.settings.last_file2.is_none());
        assert!(c.dataset2.is_none());
        drop(wake);
    }
}
