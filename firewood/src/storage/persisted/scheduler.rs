use super::{PersistedStore, StoreWrite};
use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

struct DiskScheduler {
    store: Arc<PersistedStore>, // Trait object for the persisted store
    change_queue: Mutex<VecDeque<StoreWrite>>, // Mutex to synchronize access to change_queue among threads
    writer: mpsc::Sender<Vec<StoreWrite>>,     // Sender for write requests with ack channel
}

impl DiskScheduler {
    fn new(store: Arc<PersistedStore>, change_queue: Mutex<VecDeque<StoreWrite>>) -> Self {
        let (writer_tx, writer_rx) = mpsc::channel();

        // Spawn the writer thread
        let store = Arc::clone(&store);

        thread::spawn(move || {
            for (write, ack_sender) in writer_rx {
                // Write to the store
                store.write_all(write); // Cloning the data to pass ownership

                // Send acknowledgment
                // Ignoring the result for simplicity
                let _ = ack_sender.send(()); 
            }
        });

        // Spawn a thread to consume the change_queue
        let store_clone = Arc::clone(&store);
        thread::spawn(move || {
            loop {
                if let Some(write) = change_queue.lock().unwrap().pop_front() {
                    let (ack_tx, _) = mpsc::channel();
                    writer_tx.send((write, ack_tx)).unwrap();
                }

                thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        Self {
            store,
            change_queue,
            writer: writer_tx,
        }
    }

    fn enqueue_write(&self, write: StoreWrite) {
        self.change_queue.lock().unwrap().push_back(write);
    }
}
