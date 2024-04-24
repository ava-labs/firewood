use super::{PersistedStore, StoreWrite};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, Notify};

#[derive(Debug, Error)]
pub enum StoreError<T> {
    #[error("error sending data: `{0}`")]
    Send(#[from] tokio::sync::mpsc::error::SendError<T>),
}

struct DiskRequester {
    sender: mpsc::Sender<(Vec<StoreWrite>, Arc<Notify>)>,
}

impl DiskRequester {
    const fn new(write_sender: mpsc::Sender<(Vec<StoreWrite>, Arc<Notify>)>) -> Self {
        Self {
            sender: write_sender,
        }
    }

    pub async fn send_writes(&self, writes: Vec<StoreWrite>) {
        let notifier = Arc::new(Notify::new());

        self.sender
            .send((writes, notifier.clone()))
            .await
            .map_err(StoreError::Send)
            .ok();

        // wait on the notification
        notifier.notified().await
    }
}

struct DiskScheduler {
    receiver: mpsc::Receiver<(Vec<StoreWrite>, Arc<Notify>)>,
}

impl DiskScheduler {
    const fn new(write_receiver: mpsc::Receiver<(Vec<StoreWrite>, Arc<Notify>)>) -> Self {
        Self {
            receiver: write_receiver,
        }
    }

    #[tokio::main(flavor = "current_thread")]
    // The caller (e.g. Coordinator) should spawn the writer thread.
    pub async fn run(self, store: &mut PersistedStore) {
        let mut receiver = self.receiver;
        let local_pool = tokio::task::LocalSet::new();
        local_pool
            // everything needs to be moved into this future in order to be properly dropped
            .run_until(async move {
                loop {
                    while let Some(element) = receiver.recv().await {
                        // Write to the store
                        let (writes, notifier) = element;
                        store.write_all(&writes).await.unwrap_or_else(|err| {
                            eprintln!("Error writing to store: {:?}", err);
                        });

                        // Ack the writes are complete.
                        notifier.notify_one();
                    }
                }
            })
            .await;

        // when finished process all requests, wait for any pending-futures to complete
        local_pool.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use tokio::fs;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_disk_scheduler() {
        // Create a temporary directory to store the file
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_persisted_file");

        let mut store = PersistedStore::new(file_path.clone()).await.unwrap();

        // Create the DiskScheduler and requster.
        let queue_size = 10;
        let (write_sender, write_receiver) = mpsc::channel(queue_size);
        let requester = DiskRequester::new(write_sender);
        let scheduler = DiskScheduler::new(write_receiver);

        // Enqueue some writes
        let first_data = "Hello, ";
        let second_data = "world";

        let writes = vec![
            StoreWrite {
                offset: 0,
                data: Bytes::from(first_data),
            },
            StoreWrite {
                offset: first_data.len() as u64,
                data: Bytes::from(second_data),
            },
        ];

        // Start the scheduler

        std::thread::Builder::new()
            .name("DiskBuffer".to_string())
            .spawn(move || scheduler.run(&mut store))
            .expect("thread spawn should succeed");

        // Enqueue the writes.
        requester.send_writes(writes).await;

        // Check if the writes were processed
        let store = PersistedStore::new(file_path.clone()).await.unwrap();

        let read_data = store
            .stream_from(0, first_data.len() + second_data.len())
            .await
            .unwrap();
        let read_str = String::from_utf8(read_data.to_vec()).unwrap();

        // Verify that the read data matches the expected content
        assert_eq!(read_str, first_data.to_owned() + second_data);

        // Clean up by deleting the temporary file
        fs::remove_file(file_path)
            .await
            .expect("Failed to delete temporary file");
    }
}
