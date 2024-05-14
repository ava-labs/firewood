use std::path::PathBuf;

use super::PersistedStore;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum StoreError<T> {
    #[error("error sending data: `{0}`")]
    Send(#[from] tokio::sync::mpsc::error::SendError<T>),
}

#[derive(Debug)]
/// DiskRequester is responsible for sending writes to the DiskScheduler.
pub struct DiskRequester {
    sender: mpsc::UnboundedSender<(u64, Box<[u8]>)>,
}

impl DiskRequester {
    /// Creates a new instance of `DiskRequester` with the specified sender.
    pub const fn new(write_sender: mpsc::UnboundedSender<(u64, Box<[u8]>)>) -> Self {
        Self {
            sender: write_sender,
        }
    }

    /// Sends the writes to the DiskScheduler.
    pub fn send_writes(&self, writes: (u64, Box<[u8]>)) {
        self.sender.send(writes).map_err(StoreError::Send).ok();
    }
}

#[derive(Debug)]
/// DiskScheduler is responsible for scheudling writes to the store.
pub struct DiskScheduler {
    receiver: mpsc::UnboundedReceiver<(u64, Box<[u8]>)>,
}

impl DiskScheduler {
    /// Creates a new instance of `DiskScheduler` with the specified receiver.
    pub const fn new(write_receiver: mpsc::UnboundedReceiver<(u64, Box<[u8]>)>) -> Self {
        Self {
            receiver: write_receiver,
        }
    }

    #[tokio::main(flavor = "current_thread")]

    /// run starts the DiskScheduler and processes all the writes.
    /// The caller (e.g. Coordinator) should spawn the writer thread.
    pub async fn run(self, filename: PathBuf) {
        let mut receiver = self.receiver;
        let mut store = PersistedStore::new(filename).await.unwrap();
        let local_pool = tokio::task::LocalSet::new();
        local_pool
            // everything needs to be moved into this future in order to be properly dropped
            .run_until(async move {
                loop {
                    while let Some(element) = receiver.recv().await {
                        // Write to the store
                        let writes = element;
                        store
                            .write(writes.0, &writes.1)
                            .await
                            .unwrap_or_else(|err| {
                                eprintln!("Error writing to store: {:?}", err);
                            });
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_disk_scheduler() {
        // Create a temporary directory to store the file
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_persisted_file");

        // Create the DiskScheduler and requster.
        let (write_sender, write_receiver) = mpsc::unbounded_channel();
        let requester = DiskRequester::new(write_sender);
        let scheduler = DiskScheduler::new(write_receiver);

        // Enqueue some writes
        let first_data = "Hello, ";
        let second_data = "world";

        // Start the scheduler
        let filepath = file_path.clone();
        std::thread::Builder::new()
            .name("DiskBuffer".to_string())
            .spawn(move || scheduler.run(filepath))
            .expect("thread spawn should succeed");

        // Enqueue the writes.
        requester.send_writes((0, first_data.as_bytes().to_owned().into_boxed_slice()));
        requester.send_writes((
            first_data.len() as u64,
            second_data.as_bytes().to_owned().into_boxed_slice(),
        ));

        // Check if the writes were processed
        let store = PersistedStore::new(file_path.clone()).await.unwrap();

        let read_data = store
            .stream_from(0, first_data.len() + second_data.len())
            .await
            .unwrap();
        let read_str = String::from_utf8(read_data.to_vec()).unwrap();

        // Verify that the read data matches the expected content
        assert_eq!(read_str, first_data.to_owned() + second_data);
    }
}
