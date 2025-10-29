# fwdctl

`fwdctl` is a command-line interface tool designed to make it easy to experiment with and manage Firewood databases locally. Firewood is an embedded key-value store optimized for blockchain state management, providing Merkle trie-based storage with cryptographic proofs.

## Building locally

```sh
cargo build --release --bin fwdctl
```

To use:

```sh
./target/release/fwdctl -h
```

## Table of Contents

- [Supported Commands](#supported-commands)
- [Common Workflows](#common-workflows)
- [Command Reference](#command-reference)
- [Advanced Usage](#advanced-usage)
- [Troubleshooting](#troubleshooting)
- [Performance Tips](#performance-tips)
- [Integration with Scripting](#integration-with-scripting)

## Supported Commands

* `fwdctl create`: Create a new firewood database
* `fwdctl insert`: Insert a key/value pair into the database
* `fwdctl get`: Get the value associated with a key
* `fwdctl delete`: Delete a key/value pair from the database
* `fwdctl root`: Get the root hash of the key/value trie
* `fwdctl dump`: Dump the contents of the key/value store
* `fwdctl graph`: Generate a DOT file visualization of the database structure
* `fwdctl check`: Run integrity checks on the database

## Common Workflows

### Getting Started: Create Your First Database

```sh
# Create a new database with default settings
$ fwdctl create --db mydata.db
created firewood database in mydata.db

# Insert some data
$ fwdctl insert user:1:name "Alice" --db mydata.db
user:1:name

$ fwdctl insert user:1:email "alice@example.com" --db mydata.db
user:1:email

# Retrieve the data
$ fwdctl get user:1:name --db mydata.db
"Alice"

# View all data in the database
$ fwdctl dump --db mydata.db
'user:1:email': 'alice@example.com'
'user:1:name': 'Alice'
```

### Managing User Data

```sh
# Create a database for user records
$ fwdctl create --db users.db

# Insert multiple user records
$ fwdctl insert user:1:name "Alice" --db users.db
$ fwdctl insert user:1:email "alice@example.com" --db users.db
$ fwdctl insert user:2:name "Bob" --db users.db
$ fwdctl insert user:2:email "bob@example.com" --db users.db

# Query specific user data
$ fwdctl get user:2:name --db users.db
"Bob"

# Delete a user's email
$ fwdctl delete user:2:email --db users.db
key user:2:email deleted successfully

# Verify the deletion
$ fwdctl dump --db users.db
'user:1:email': 'alice@example.com'
'user:1:name': 'Alice'
'user:2:name': 'Bob'
```

### Exporting Data

```sh
# Export to CSV format
$ fwdctl dump --db mydata.db --output-format csv --output-file-name backup
Dumping to backup.csv

# Export to JSON format
$ fwdctl dump --db mydata.db --output-format json --output-file-name backup
Dumping to backup.json

# Export with hex-encoded keys and values
$ fwdctl dump --db mydata.db --hex
'757365723a313a656d61696c': '616c696365406578616d706c652e636f6d'
'757365723a313a6e616d65': '416c696365'
```

### Verifying Database Integrity

```sh
# Get the root hash (Merkle root) of the database
$ fwdctl root --db mydata.db
Some(0c7b73e30a80e925e73c1f60b3aa663cbf7d35b777ee2c8adccf47834135a7b7)

# Run a basic integrity check
$ fwdctl check --db mydata.db

# Run a comprehensive check including hash verification
$ fwdctl check --db mydata.db --hash-check

# Check and automatically fix issues
$ fwdctl check --db mydata.db --fix
```

## Command Reference

### `fwdctl create`

Create a new firewood database.

**Usage:**
```sh
fwdctl create [OPTIONS]
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)
- `--truncate`: Truncate and reset the database if it exists (default: true)
- `--file-nbit <WAL_FILE_NBIT>`: Size of WAL file in bits (default: 22)
- `--max-revisions <MAX_REVISIONS>`: Number of historical revisions to keep (default: 100)

**Examples:**
```sh
# Create with default settings
$ fwdctl create

# Create with custom name
$ fwdctl create --db blockchain.db

# Create without truncating existing database
$ fwdctl create --db mydata.db --truncate false

# Create with custom revision history
$ fwdctl create --db mydata.db --max-revisions 200
```

### `fwdctl insert`

Insert a key/value pair into the database.

**Usage:**
```sh
fwdctl insert [OPTIONS] <KEY> <VALUE>
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)

**Arguments:**
- `<KEY>`: The key to insert
- `<VALUE>`: The value to associate with the key

**Examples:**
```sh
# Insert a simple key-value pair
$ fwdctl insert username "alice"
username

# Insert with structured keys
$ fwdctl insert config:server:port "8080" --db app.db

# Insert with custom database
$ fwdctl insert balance:0x123 "1000000" --db ledger.db
```

**Output:**
The command outputs the inserted key on success.

### `fwdctl get`

Get the value associated with a key.

**Usage:**
```sh
fwdctl get [OPTIONS] <KEY>
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)

**Arguments:**
- `<KEY>`: The key to retrieve

**Examples:**
```sh
# Get a value
$ fwdctl get username
"alice"

# Get from specific database
$ fwdctl get config:server:port --db app.db
"8080"
```

**Output:**
- Success: Returns the value in quoted string format
- Key not found: `Key 'keyname' not found`
- Database empty: `Database is empty`

### `fwdctl delete`

Delete a key/value pair from the database.

**Usage:**
```sh
fwdctl delete [OPTIONS] <KEY>
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)

**Arguments:**
- `<KEY>`: The key to delete

**Examples:**
```sh
# Delete a key
$ fwdctl delete username
key username deleted successfully

# Delete from specific database
$ fwdctl delete user:1:email --db users.db
key user:1:email deleted successfully
```

**Output:**
Confirms successful deletion with message: `key <KEY> deleted successfully`

### `fwdctl root`

Display the root hash of the key/value trie (Merkle root).

**Usage:**
```sh
fwdctl root [OPTIONS]
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)

**Examples:**
```sh
# Get root hash
$ fwdctl root
Some(0c7b73e30a80e925e73c1f60b3aa663cbf7d35b777ee2c8adccf47834135a7b7)

# Get root hash from specific database
$ fwdctl root --db mydata.db
Some(a8b3c9d7e2f1a4b6c8d9e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5)
```

**Output:**
- Database with data: `Some(<64-character-hex-hash>)`
- Empty database: `None`

### `fwdctl dump`

Dump the contents of the key/value store.

**Usage:**
```sh
fwdctl dump [OPTIONS]
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)
- `-s, --start-key <START_KEY>`: Start dumping from this key (inclusive)
- `-S, --stop-key <STOP_KEY>`: Stop dumping at this key (inclusive)
- `--start-key-hex <START_KEY_HEX>`: Start key in hex format
- `--stop-key-hex <STOP_KEY_HEX>`: Stop key in hex format
- `-m, --max-key-count <MAX_KEY_COUNT>`: Maximum number of keys to dump
- `-o, --output-format <OUTPUT_FORMAT>`: Output format (stdout, csv, json, dot)
- `-f, --output-file-name <OUTPUT_FILE_NAME>`: Output file name (default: `dump`)
- `-x, --hex`: Print keys and values in hex format

**Examples:**
```sh
# Dump all data to stdout
$ fwdctl dump
'key1': 'value1'
'key2': 'value2'

# Dump to CSV file
$ fwdctl dump --output-format csv --output-file-name export
Dumping to export.csv

# Dump to JSON file
$ fwdctl dump --output-format json
Dumping to dump.json

# Dump with key range
$ fwdctl dump --start-key "user:100" --stop-key "user:200"

# Dump limited number of entries
$ fwdctl dump --max-key-count 100

# Dump in hex format
$ fwdctl dump --hex
'757365723a31': '416c696365'

# Resume dumping from specific key
$ fwdctl dump --start-key "user:500" --max-key-count 100
```

**Output Formats:**
- **stdout**: Human-readable format (default)
- **csv**: Comma-separated values file
- **json**: JSON object with key-value pairs
- **dot**: GraphViz DOT format for visualization

### `fwdctl graph`

Generate a DOT file visualization of the database structure.

**Usage:**
```sh
fwdctl graph [OPTIONS]
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)

**Examples:**
```sh
# Generate graph visualization
$ fwdctl graph --db mydata.db > database.dot

# Convert to PNG using GraphViz
$ dot -Tpng database.dot -o database.png
```

### `fwdctl check`

Run integrity checks on the database.

**Usage:**
```sh
fwdctl check [OPTIONS]
```

**Options:**
- `-d, --db <DB_NAME>`: Database path (default: `firewood.db`)
- `--hash-check`: Perform cryptographic hash verification
- `--fix`: Attempt to fix observed inconsistencies

**Examples:**
```sh
# Basic integrity check
$ fwdctl check

# Comprehensive check with hash verification
$ fwdctl check --hash-check

# Check and attempt to fix issues
$ fwdctl check --fix
```

**Output:**
The check command provides detailed statistics including:
- Error count and descriptions
- Database size and key-value counts
- Trie statistics (branching factors, depths)
- Memory usage (branch/leaf area statistics)
- Internal fragmentation analysis

## Advanced Usage

### Partial Dumps with Key Ranges

When working with large databases, you can dump data in chunks:

```sh
# Dump first 1000 keys
$ fwdctl dump --max-key-count 1000
...
Next key is user:1000, resume with "--start-key=user:1000" or "--start-key-hex=757365723a31303030".

# Continue from where you left off
$ fwdctl dump --start-key "user:1000" --max-key-count 1000
```

### Working with Hex Keys

For binary or non-UTF8 keys, use hex encoding:

```sh
# Insert with hex key
$ fwdctl insert $(echo -n "binary_key" | xxd -p) "value"

# Retrieve with hex
$ fwdctl dump --start-key-hex "62696e6172795f6b6579"

# Dump everything in hex format
$ fwdctl dump --hex --output-format json
```

### Database Visualization

Generate visual representations of your database structure:

```sh
# Create a DOT file
$ fwdctl graph --db mydata.db > db_structure.dot

# Convert to image formats using GraphViz
$ dot -Tpng db_structure.dot -o structure.png
$ dot -Tsvg db_structure.dot -o structure.svg
$ dot -Tpdf db_structure.dot -o structure.pdf
```

### Comprehensive Database Health Check

```sh
# Run full check with hash verification
$ fwdctl check --db production.db --hash-check

# Review the output for:
# - Leaked areas (memory fragmentation)
# - Hash inconsistencies
# - Storage overhead metrics
# - Internal fragmentation percentages
```

## Troubleshooting

### Common Errors and Solutions

#### Error: "No such file or directory"

```
Error: FileIO(FileIoError { inner: Os { code: 2, kind: NotFound, message: "No such file or directory" }
```

**Cause:** The specified database file doesn't exist.

**Solution:**
```sh
# Create the database first
$ fwdctl create --db mydata.db

# Or check if you're using the correct path
$ ls -la *.db
```

#### Error: "Key not found"

```
Key 'username' not found
```

**Cause:** The requested key doesn't exist in the database.

**Solution:**
```sh
# Verify the key exists
$ fwdctl dump --db mydata.db

# Check for typos in the key name
$ fwdctl dump | grep username
```

#### Error: Database is empty

```
Database is empty
```

**Cause:** Attempting to read from a newly created or truncated database.

**Solution:**
```sh
# Insert data first
$ fwdctl insert key "value" --db mydata.db

# Or check if you truncated accidentally
$ fwdctl create --db mydata.db --truncate false
```

#### Error: Conflicting options

```
error: the argument '--start-key <START_KEY>' cannot be used with '--start-key-hex <START_KEY_HEX>'
```

**Cause:** Using both regular and hex versions of the same option.

**Solution:**
```sh
# Use only one format
$ fwdctl dump --start-key "user:1"
# OR
$ fwdctl dump --start-key-hex "757365723a31"
```

#### Integrity Check Failures

If `fwdctl check` reports errors:

```sh
# First, try basic check
$ fwdctl check --db mydata.db

# If errors found, try to fix them
$ fwdctl check --db mydata.db --fix

# Review what was fixed
# Check output for "Fixed Errors" and "Unfixable Errors"
```

**Note:** Always backup your database before running `--fix`.

### Database Recovery

If your database is corrupted:

1. **Create a backup** (if possible):
   ```sh
   $ cp mydata.db mydata.db.backup
   ```

2. **Attempt automatic fix**:
   ```sh
   $ fwdctl check --db mydata.db --fix
   ```

3. **Export salvageable data**:
   ```sh
   $ fwdctl dump --db mydata.db --output-format json > recovery.json
   ```

4. **Create a fresh database and re-import**:
   ```sh
   $ fwdctl create --db mydata_new.db
   # Re-insert data from recovery.json using scripts
   ```

## Performance Tips

### For Large Databases

1. **Use batch operations**: When inserting multiple keys, consider writing a script that batches inserts to minimize overhead.

2. **Optimize dump operations**:
   ```sh
   # Export in chunks for very large databases
   $ fwdctl dump --max-key-count 10000 --output-format csv
   
   # Use specific key ranges to target data
   $ fwdctl dump --start-key "range_start" --stop-key "range_end"
   ```

3. **Configure WAL appropriately**:
   ```sh
   # For write-heavy workloads, increase WAL size
   $ fwdctl create --file-nbit 24 --max-revisions 50
   ```

4. **Regular integrity checks**:
   ```sh
   # Schedule periodic checks (without --hash-check for faster checks)
   $ fwdctl check --db production.db
   ```

5. **Monitor database statistics**:
   - Check output from `fwdctl check` for storage overhead
   - Look for high internal fragmentation percentages
   - Monitor leaked areas that indicate memory issues

### Memory Considerations

- The `check` command can be memory-intensive for large databases
- Use `--hex` output only when necessary, as it increases output size
- For very large dumps, prefer CSV or JSON formats over stdout

## Integration with Scripting

### Bash Script Examples

#### Bulk Insert Script

```bash
#!/bin/bash
# bulk_insert.sh - Insert multiple key-value pairs

DB="mydata.db"

# Create database
fwdctl create --db "$DB"

# Read from CSV and insert
while IFS=',' read -r key value; do
    fwdctl insert "$key" "$value" --db "$DB"
    echo "Inserted: $key"
done < data.csv
```

#### Backup Script

```bash
#!/bin/bash
# backup.sh - Create timestamped database backups

DB="production.db"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="backups"

mkdir -p "$BACKUP_DIR"

# Export data
fwdctl dump --db "$DB" --output-format json --output-file-name "$BACKUP_DIR/backup_$TIMESTAMP"

# Get root hash for verification
ROOT=$(fwdctl root --db "$DB")
echo "$ROOT" > "$BACKUP_DIR/backup_${TIMESTAMP}_root.txt"

echo "Backup completed: $BACKUP_DIR/backup_${TIMESTAMP}.json"
```

#### Health Check Script

```bash
#!/bin/bash
# health_check.sh - Monitor database health

DB="$1"

if [ -z "$DB" ]; then
    echo "Usage: $0 <database_path>"
    exit 1
fi

echo "Running health check on $DB..."
OUTPUT=$(fwdctl check --db "$DB" 2>&1)

# Check for errors
if echo "$OUTPUT" | grep -q "Errors (0):"; then
    echo "✓ Database is healthy"
    exit 0
else
    echo "✗ Database has errors:"
    echo "$OUTPUT" | grep "Errors"
    exit 1
fi
```

### Python Integration Example

```python
#!/usr/bin/env python3
"""
Simple Python wrapper for fwdctl operations
"""
import subprocess
import json

class FirewoodDB:
    def __init__(self, db_path="firewood.db"):
        self.db_path = db_path
        self.fwdctl = "fwdctl"  # Assumes fwdctl is in PATH
    
    def create(self):
        """Create a new database"""
        subprocess.run([self.fwdctl, "create", "--db", self.db_path], check=True)
    
    def insert(self, key, value):
        """Insert a key-value pair"""
        subprocess.run([self.fwctl, "insert", key, value, "--db", self.db_path], check=True)
    
    def get(self, key):
        """Get a value by key"""
        result = subprocess.run(
            [self.fwdctl, "get", key, "--db", self.db_path],
            capture_output=True,
            text=True
        )
        return result.stdout.strip().strip('"')
    
    def delete(self, key):
        """Delete a key"""
        subprocess.run([self.fwdctl, "delete", key, "--db", self.db_path], check=True)
    
    def dump_json(self, output_file="dump.json"):
        """Dump database to JSON"""
        subprocess.run([
            self.fwdctl, "dump",
            "--db", self.db_path,
            "--output-format", "json",
            "--output-file-name", output_file.replace('.json', '')
        ], check=True)
        
        with open(output_file, 'r') as f:
            return json.load(f)
    
    def root_hash(self):
        """Get the root hash"""
        result = subprocess.run(
            [self.fwdctl, "root", "--db", self.db_path],
            capture_output=True,
            text=True
        )
        return result.stdout.strip()

# Example usage
if __name__ == "__main__":
    db = FirewoodDB("example.db")
    db.create()
    db.insert("user:1", "Alice")
    db.insert("user:2", "Bob")
    
    print(f"user:1 = {db.get('user:1')}")
    print(f"Root hash: {db.root_hash()}")
    
    data = db.dump_json()
    print(f"All data: {json.dumps(data, indent=2)}")
```

### Exit Codes

`fwdctl` follows standard Unix exit code conventions:
- `0`: Success
- `1`: Error occurred

Use this in scripts for error handling:

```bash
if fwdctl get mykey --db mydata.db > /dev/null 2>&1; then
    echo "Key exists"
else
    echo "Key not found or error occurred"
fi
```

## Additional Resources

- [Firewood Main Documentation](../README.md)
- [Firewood API Documentation](../firewood/README.md)
- GraphViz Documentation: https://graphviz.org/

## Contributing

For questions, issues, or contributions related to `fwdctl`, please refer to the main Firewood repository.
