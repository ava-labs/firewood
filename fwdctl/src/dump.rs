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

    /// The key to start dumping from (if no key is provided, start from the beginning) in hex format.
    /// Defaults to None.
    #[arg(
        long,
        required = false,
        conflicts_with = "start_key",
        value_name = "START_KEY_HEX",
        value_parser = key_parser,
        help = "Start dumping from this key (inclusive) in hex format. Conflicts with start_key"
    )]
    pub start_key_hex: Option<Key>,

    /// The key to stop dumping to (if no key is provided, stop to the end) in hex format.
    /// Defaults to None.
    #[arg(
        long,
        required = false,
        conflicts_with = "stop_key",
        value_name = "STOP_KEY_HEX",
        value_parser = key_parser,
        help = "Stop dumping to this key (inclusive) in hex format. Conflicts with stop_key"
    )]
    pub stop_key_hex: Option<Key>,

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

    /// The output file name of database dump.
    /// Output format must be set when the file name is set.
    #[arg(
        short = 'f',
        long,
        requires = "output_format",
        value_name = "OUTPUT_FILE_NAME",
        default_value = "dump",
        help = "Output file name of database dump, default to dump. Output format must be set when the file name is set."
    )]
    pub output_file_name: String,

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

    let start_key = opts
        .start_key_hex
        .clone()
        .and_then(|start_key_hex| parse_hex_key(start_key_hex).ok())
        .or_else(|| opts.start_key.clone())
        .unwrap_or_else(|| Box::new([]));
    let stop_key = opts
        .stop_key_hex
        .clone()
        .and_then(|stop_key_hex| parse_hex_key(stop_key_hex).ok())
        .or_else(|| opts.stop_key.clone())
        .unwrap_or_else(|| Box::new([]));
    let mut key_count = 0;

    let mut stream = MerkleKeyValueStream::from_key(&latest_rev, start_key);
    let mut output_handler = create_output_handler(opts).expect("Error creating output handler");

    while let Some(item) = stream.next().await {
        match item {
            Ok((key, value)) => {
                output_handler.handle_record(&key, &value)?;

                key_count += 1;

                if (!stop_key.is_empty() && key >= stop_key)
                    || key_count_exceeded(opts.max_key_count, key_count)
                {
                    handle_next_key(stream.next().await).await;
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    output_handler.flush()?;

    Ok(())
}

fn key_count_exceeded(max: Option<u32>, key_count: u32) -> bool {
    max.map(|max| key_count >= max).unwrap_or(false)
}

fn u8_to_string(data: &[u8]) -> String {
    String::from_utf8_lossy(data).to_string()
}

fn key_parser(s: &str) -> Result<Box<[u8]>, std::io::Error> {
    Ok(Box::from(s.as_bytes()))
}

fn parse_hex_key(hex_str: Box<[u8]>) -> Result<Key, Box<dyn Error>> {
    hex::decode(String::from_utf8_lossy(&hex_str).as_bytes())
        .map(|decoded| decoded.into_boxed_slice())
        .map_err(|e| e.into())
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
                "Next key is {0}, resume with \"--start-key={0}\" or \"--start-key-hex={1}\".",
                u8_to_string(&key),
                hex::encode(&key)
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
    fn handle_record(&mut self, key: &[u8], value: &[u8]) -> Result<(), std::io::Error>;
    fn flush(&mut self) -> Result<(), std::io::Error>;
}

struct CsvOutputHandler {
    writer: csv::Writer<File>,
    hex: bool,
}

impl OutputHandler for CsvOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) -> Result<(), std::io::Error> {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        self.writer.write_record(&[key_str, value_str])?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.writer.flush()?;
        Ok(())
    }
}

struct JsonOutputHandler {
    writer: BufWriter<File>,
    hex: bool,
    is_first: bool,
}

impl OutputHandler for JsonOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) -> Result<(), std::io::Error> {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        if self.is_first {
            self.writer.write_all(b"{\n")?;
            self.is_first = false;
        } else {
            self.writer.write_all(b",\n")?;
        }

        write!(self.writer, r#"  "{key_str}": "{value_str}""#)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        let _ = self.writer.write(b"\n}\n");
        self.writer.flush()?;
        Ok(())
    }
}

struct StdoutOutputHandler {
    hex: bool,
}

impl OutputHandler for StdoutOutputHandler {
    fn handle_record(&mut self, key: &[u8], value: &[u8]) -> Result<(), std::io::Error> {
        let (key_str, value_str) = key_value_to_string(key, value, self.hex);
        println!("'{}': '{}'", key_str, value_str);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

fn create_output_handler(
    opts: &Options,
) -> Result<Box<dyn OutputHandler + Send + Sync>, Box<dyn Error>> {
    let hex = opts.hex;
    match opts.output_format.as_str() {
        "csv" => {
            let file_name = format!("{}.csv", opts.output_file_name);
            println!("Dumping to {}", file_name);
            let file = File::create(file_name)?;
            Ok(Box::new(CsvOutputHandler {
                writer: csv::Writer::from_writer(file),
                hex,
            }))
        }
        "json" => {
            let file_name = format!("{}.json", opts.output_file_name);
            println!("Dumping to {}", file_name);
            let file = File::create(file_name)?;
            Ok(Box::new(JsonOutputHandler {
                writer: BufWriter::new(file),
                hex,
                is_first: true,
            }))
        }
        "stdout" => Ok(Box::new(StdoutOutputHandler { hex })),
        _ => Err("Unknown output format".into()),
    }
}
