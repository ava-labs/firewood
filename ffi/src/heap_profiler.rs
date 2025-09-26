#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::panic;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime};

pub struct HeapReporter {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl HeapReporter {
    pub fn start(period: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = thread::spawn(move || {
            // long living profiler
            let _profiler = dhat::Profiler::new_heap();

            let mut prev_total_bytes: u64 = 0;
            let mut prev_total_blocks: u64 = 0;
            let mut have_prev = false;

            loop {
                thread::sleep(period);
                if stop2.load(Ordering::Relaxed) {
                    break;
                }

                let stats = match panic::catch_unwind(|| dhat::HeapStats::get()) {
                    Ok(s) => s,
                    Err(_) => break,
                };

                let ts = SystemTime::now();

                let (d_bytes, d_blocks) = if have_prev {
                    (
                        stats.total_bytes.saturating_sub(prev_total_bytes),
                        stats.total_blocks.saturating_sub(prev_total_blocks),
                    )
                } else {
                    (stats.total_bytes, stats.total_blocks)
                };

                eprintln!(
                    "@DHAT [dhat {:?}] curr={} bytes ({} blocks), max={} bytes; +{} bytes / +{} blocks since last",
                    ts, stats.curr_bytes, stats.curr_blocks, stats.max_bytes, d_bytes, d_blocks
                );

                prev_total_bytes = stats.total_bytes;
                prev_total_blocks = stats.total_blocks;
                have_prev = true;
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

// impl Drop for HeapReporter {
//     fn drop(&mut self) {
//         self.stop.store(true, Ordering::Relaxed);
//         if let Some(h) = self.handle.take() {
//             let _ = h.join();
//         }
//     }
// }
