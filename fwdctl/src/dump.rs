// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::db::{Db, DbConfig};
use firewood::merkle::Key;
use firewood::stream::MerkleKeyValueStream;
use firewood::v2::api::{self, Db as _};
use futures_util::StreamExt;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

type KeyFromStream = Option<Result<(Box<[u8]>, Vec<u8>), api::Error>>;

#[derive(Debug, Args)]
pub struct Options {
    /// The database path (if no path is provided, return an error). Defaults to firewood.
    #[arg(
        required = true,
        value_name = "DB_NAME",
        default_value_t = String::from("firewood"),
        help = "Name of the database"
    )]
    pub db: String,

    /// The key to start dumping from (if no key is provided, start from the beginning).
    /// Defaults to None.
    #[arg(
        short = 's',
        long,
        required = false,
        value_name = "START_KEY",
        value_parser = key_parser,
        help = "Start dumping from this key (inclusive)."
    )]
    pub start_key: Option<Key>,

    /// The key to stop dumping to (if no key is provided, stop to the end).
    /// Defaults to None.
    #[arg(
        short = 'S',
        long,
        required = false,
        value_name = "STOP_KEY",
        value_parser = key_parser,
        help = "Stop dumping to this key (inclusive)."
    )]
    pub stop_key: Option<Key>,

    /// The max number of the keys going to be dumped.
    /// Defaults to None.
    #[arg(
        short = 'm',
        long,
        required = false,
        value_name = "MAX_KEY_COUNT",
        help = "Maximum number of keys going to be dumped."
    )]
    pub max_key_count: Option<u32>,

    /// The output format of database dump.
    /// Possible Values: ["csv", "json", "stdout"].
    /// Defaults to "stdout"
    #[arg(
        short = 'o',
        long,
        required = false,
        value_name = "OUTPUT_FORMAT",
        value_parser = ["csv", "json", "stdout"],
        default_value = "stdout",
        help = "Output format of database dump, default to stdout. CSV and JSON formats are available."
    )]
    pub output_format: String,

    #[arg(short = 'x', long, help = "Print the keys and values in hex format.")]
    pub hex: bool,
}

pub(super) async fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("dump database {:?}", opts);

    let cfg = DbConfig::builder().truncate(false);
    let db = Db::new(opts.db.clone(), cfg.build()).await?;
    let latest_hash = db.root_hash().await?;
    let Some(latest_hash) = latest_hash else {
        println!("Database is empty");
        return Ok(());
    };
    let latest_rev = db.revision(latest_hash).await?;

    let start_key = opts.start_key.clone().unwrap_or_else(|| Box::new([]));
    let stop_key = opts.stop_key.clone().unwrap_or_else(|| Box::new([]));
    let mut key_count = 0;

    let mut stream = MerkleKeyValueStream::from_key(&latest_rev, start_key);
    let mut output_handler = create_output_handler(opts).expect("Error creating output handler");

    while let Some(item) = stream.next().await {
        match item {
            Ok((key, value)) => {
                output_handler.handle_record(&key, &value);

                key_count += 1;

                if key == stop_key || key_count_exceeded(opts.max_key_count, key_count) {
                    handle_next_key(stream.next().await).await;
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    output_handler.flush();

    Ok(())
}

const fn key_count_exceeded(max: Option<u32>, key_count: u32) -> bool {
    matches!(max, Some(max) if key_count >= max)
}

fn u8_to_string(data: &[u8]) -> String {
    String::from_utf8_lossy(data).to_string()
}

fn key_parser(s: &str) -> Result<Box<[u8]>, std::io::Error> {
    Ok(Box::from(s.as_bytes()))
}

// Helper function to convert key and value to a string
fn key_value_to_string(key: &[u8], value: &[u8], hex: bool) -> (String, String) {
    let key_str = if hex {
        hex::encode(key)
    } else {
        u8_to_string(key)
    };
    let value_str = if hex {
        hex::encode(value)
    } else {
        u8_to_string(value)
    };
    (key_str, value_str)
}

async fn handle_next_key(next_key: KeyFromStream) {
    match next_key {
        Some(Ok((key, _))) => {
            println!(
                "Next key is {0}, resume with \"--start-key={0}\".",
                u8_to_string(&key)
            );
        }
        Some(Err(e)) => {
            eprintln!("Error occurred while fetching the next key: {}", e);
        }
        None => {
            println!("There is no next key. Data dump completed.");
        }
    }
}

trait OutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]);
    fn flush(&mut self);
}

struct CsvOutputHandler {
    writer: csv::Writer<File>,
    hex: bool,
}

impl OutputHandler for CsvOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        self.writer
            .write_record(&[key_str, value_str])
            .expect("Failed to write CSV record");
    }

    fn flush(&mut self) {
        self.writer.flush().expect("Failed to flush CSV writer");
    }
}

struct JsonOutputHandler {
    writer: BufWriter<File>,
    hex: bool,
    is_first: bool,
}

impl JsonOutputHandler {
    fn new(mut writer: BufWriter<File>, hex: bool) -> JsonOutputHandler {
        let _ = writer.write("{\n".as_bytes());
        JsonOutputHandler {
            writer,
            hex,
            is_first: true,
        }
    }
}

impl OutputHandler for JsonOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        if self.is_first {
            let _ = self
                .writer
                .write(format!("  \"{}\": \"{}\"", key_str, value_str).as_bytes());
            self.is_first = false;
        } else {
            let _ = self
                .writer
                .write(format!(",\n  \"{}\": \"{}\"", key_str, value_str).as_bytes());
        }
    }

    fn flush(&mut self) {
        let _ = self.writer.write("\n}\n".as_bytes());
        self.writer
            .flush()
            .expect("Failed to flush Json Buf writer");
    }
}

struct StdoutOutputHandler {
    hex: bool,
}

impl OutputHandler for StdoutOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        println!("'{}': '{}'", key_str, value_str);
    }
    fn flush(&mut self) {}
}

fn create_output_handler(
    opts: &Options,
) -> Result<Box<dyn OutputHandler + Send + Sync>, Box<dyn Error>> {
    let hex = opts.hex;
    match opts.output_format.as_str() {
        "csv" => {
            let file = File::create("dump.csv")?;
            Ok(Box::new(CsvOutputHandler {
                writer: csv::Writer::from_writer(file),
                hex,
            }))
        }
        "json" => {
            let file = File::create("dump.json")?;
            Ok(Box::new(JsonOutputHandler::new(BufWriter::new(file), hex)))
        }
        "stdout" => Ok(Box::new(StdoutOutputHandler { hex })),
        _ => Err("Unknown output format".into()),
    }
}
