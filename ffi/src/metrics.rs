use std::{
    fs::File,
    io::Write,
    ops::Deref,
    path::Path,
    sync::{atomic::Ordering, Arc, Mutex, Once},
    thread::sleep,
};

use chrono::Utc;

use metrics::Key;
use metrics_util::registry::{AtomicStorage, Registry};

static INIT: Once = Once::new();

pub(crate) fn setup_metrics(file: &Path, metric_interval_secs: u32) {
    INIT.call_once(|| {
        let file = File::create(file).expect("failed to create metrics file");
        let inner = TextRecorderInner {
            registry: Registry::atomic(),
            file: file.into(),
        };
        let recorder = TextRecorder {
            inner: Arc::new(inner),
        };
        metrics::set_global_recorder(recorder.clone()).expect("failed to set recorder");
        std::thread::spawn(move || loop {
            sleep(std::time::Duration::from_secs(metric_interval_secs as u64));
            let mut guard = recorder.inner.file.lock().expect("poisoned");
            writeln!(guard, "{}", Utc::now()).unwrap();
            let counters = recorder.registry.get_counter_handles();
            for (key, counter) in counters {
                writeln!(guard, "{} = {}", key, counter.load(Ordering::Relaxed)).unwrap();
            }
            writeln!(guard).unwrap();
        });
    });
}

#[derive(Debug)]
struct TextRecorderInner {
    registry: Registry<Key, AtomicStorage>,
    file: Mutex<File>,
}

#[derive(Debug, Clone)]
struct TextRecorder {
    inner: Arc<TextRecorderInner>,
}

impl Deref for TextRecorder {
    type Target = Arc<TextRecorderInner>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl metrics::Recorder for TextRecorder {
    fn describe_counter(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn describe_gauge(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn describe_histogram(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn register_counter(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Counter {
        self.inner
            .registry
            .get_or_create_counter(key, |c| c.clone().into())
    }

    fn register_gauge(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Gauge {
        self.inner
            .registry
            .get_or_create_gauge(key, |c| c.clone().into())
    }

    fn register_histogram(
        &self,
        key: &metrics::Key,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        self.inner
            .registry
            .get_or_create_histogram(key, |c| c.clone().into())
    }
}
