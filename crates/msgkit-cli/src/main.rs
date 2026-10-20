//! `msgkit`: convert message exports into normalized NDJSON.
//!
//! Events go to stdout (one JSON object per line); warnings go to stderr, so
//! the output can be piped straight into other tools.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::limits::Limits;
use msgkit_core::store::{write_ndjson, EraseOptions, EraseStore};
use msgkit_core::whatsapp_export::{self, DateOrder, WhatsAppExportOptions};
use msgkit_core::{ImportReport, SCHEMA_VERSION};
use msgkit_store_sqlite::{
    cloud_synced_folder, encrypt_database, key_file_too_open, DbKey, SqliteStore, StoreOptions,
};

#[derive(Parser)]
#[command(name = "msgkit", version, about = "Convert message exports to normalized NDJSON")]
struct Cli {
    /// Exit with an error if the import produced any warnings.
    #[arg(long, global = true)]
    fail_on_warnings: bool,

    /// Largest accepted input in bytes (default 33554432, 32 MiB; at least 1). Applies to each file.
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..))]
    max_input_bytes: Option<u64>,

    /// Longest accepted line of a WhatsApp export in bytes (default 262144; at least 1).
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..))]
    max_line_bytes: Option<u64>,

    /// Most messages, calls and notices accepted per file (default 1000000; at least 1).
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..))]
    max_messages: Option<u64>,

    /// Deepest accepted JSON nesting (default 32; at least 1).
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..))]
    max_json_depth: Option<u64>,

    /// Switch every input limit off. Only for files you trust: parsing can use many times the file size in memory.
    #[arg(long, global = true)]
    no_limits: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum OrderArg {
    Auto,
    Dmy,
    Mdy,
}

impl From<OrderArg> for DateOrder {
    fn from(o: OrderArg) -> Self {
        match o {
            OrderArg::Auto => DateOrder::Auto,
            OrderArg::Dmy => DateOrder::Dmy,
            OrderArg::Mdy => DateOrder::Mdy,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Import a WhatsApp "Export chat" .txt file (use `-` for stdin).
    WhatsappExport {
        file: PathBuf,
        /// Your own name as shown in the chat; sets message direction.
        #[arg(long)]
        self_name: Option<String>,
        #[arg(long, value_enum, default_value = "auto")]
        date_order: OrderArg,
        /// Phone's UTC offset in minutes, e.g. 330 for UTC+05:30.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        utc_offset_minutes: i32,
        #[arg(long)]
        conversation_id: Option<String>,
        /// Calling code, e.g. 94, for national-format phone numbers.
        #[arg(long)]
        default_country_code: Option<String>,
        /// IANA time zone of the phone, e.g. Asia/Colombo; replaces --utc-offset-minutes.
        #[arg(long)]
        time_zone: Option<String>,
    },
    /// Import Facebook Messenger `message_N.json` files.
    FacebookExport {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        self_name: Option<String>,
    },
    /// Normalize a JSON batch of Android SMS / call-log rows (`-` for stdin).
    Device {
        file: PathBuf,
        #[arg(long)]
        own_number: Option<String>,
        #[arg(long)]
        default_country_code: Option<String>,
    },
    /// Maintain a msgkit SQLite database (see `msgkit store --help`).
    Store {
        #[command(subcommand)]
        action: StoreAction,
    },
    /// Print version information.
    Version,
}

#[derive(Subcommand)]
enum StoreAction {
    /// Erase every held (not yet acknowledged) event that involves one person. Prints only a count.
    ///
    /// The address is matched the way imports normalise it: phone numbers in any spelling match when
    /// --default-country-code is the one used at import. Pass the address with --address-stdin to keep it out of
    /// the shell history and the process list.
    Erase {
        /// The database file (a local path, never a cloud-synced folder).
        db: PathBuf,
        /// The person's phone number, email address or id.
        #[arg(long, conflicts_with = "address_stdin")]
        address: Option<String>,
        /// Read the address from the first line of standard input (cannot be combined with --key-stdin).
        #[arg(long, conflicts_with = "key_stdin")]
        address_stdin: bool,
        /// Calling code, e.g. 94, the one the events were imported with.
        #[arg(long)]
        default_country_code: Option<String>,
        /// Also forget the ids of the erased events (a redelivered message would then be accepted again).
        #[arg(long)]
        drop_ids: bool,
        /// Rebuild the database file afterwards (VACUUM) so no free pages remain.
        #[arg(long)]
        compact: bool,
        #[command(flatten)]
        key: KeyArgs,
    },
    /// Encrypt a plain database once (needs a build with the `sqlcipher` feature).
    ///
    /// Stop the service and MAKE A BACKUP of the plain database first. This writes a NEW encrypted file, checks
    /// that it opens with the key and holds the same rows, and only then overwrites and deletes the plain file
    /// (unless --keep-plain). Keep the key separately from the database: without it the data cannot be recovered.
    /// Nothing is converted automatically when a database is opened.
    ///
    /// Deleting the plain file cannot guarantee that its old bytes are gone from the disk, above all on an SSD
    /// (wear levelling), a snapshot or a backup: use disk or volume encryption as well.
    Encrypt {
        /// The plain database file.
        db: PathBuf,
        /// Where to write the encrypted copy (must not exist yet). Point MSGKIT_DB_PATH at it afterwards.
        #[arg(long)]
        output: PathBuf,
        /// Keep the plain file instead of overwriting and deleting it after the copy was verified.
        #[arg(long)]
        keep_plain: bool,
        #[command(flatten)]
        key: KeyArgs,
    },
}

/// Where the database key comes from. Never an argument: a command line is visible in process listings and shell
/// history.
#[derive(clap::Args)]
struct KeyArgs {
    /// A file holding the key: 64 hexadecimal characters (for example `openssl rand -hex 32 > key`). Without this,
    /// MSGKIT_DB_KEY_FILE names the file.
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// Read the key from the first line of standard input.
    #[arg(long, conflicts_with = "key_file")]
    key_stdin: bool,
}

impl KeyArgs {
    /// The key, or `None` when none was given. Errors name the setting, never the key.
    fn read(&self) -> Result<Option<DbKey>, String> {
        let text = if self.key_stdin {
            let mut line = String::new();
            io::stdin()
                .read_line(&mut line)
                .map_err(|_| "cannot read the key from standard input".to_owned())?;
            line
        } else {
            let path = match &self.key_file {
                Some(path) => path.clone(),
                None => match std::env::var_os("MSGKIT_DB_KEY_FILE").filter(|v| !v.is_empty()) {
                    Some(path) => PathBuf::from(path),
                    None => return Ok(None),
                },
            };
            if key_file_too_open(&path) {
                eprintln!(
                    "warning: the key file can be read by its group or by other users: restrict it to its owner \
                     (chmod 600)"
                );
            }
            fs::read_to_string(path).map_err(|_| "the key file cannot be read".to_owned())?
        };
        DbKey::from_hex(&text)
            .map(Some)
            .map_err(|_| "the key must be 64 hexadecimal characters (256 bits)".to_owned())
    }
}

/// Warns on stderr when the database sits in a cloud-synced folder.
fn warn_if_synced(db: &Path) {
    if let Some(provider) = cloud_synced_folder(db) {
        eprintln!(
            "warning: the database is inside a {provider} folder; the sync copies it and its write-ahead log where \
             msgkit cannot erase them. Use a local path that is not synced"
        );
    }
}

/// Runs a `store` action. Returns the line to print; errors never include the address.
fn run_store(action: StoreAction) -> Result<String, String> {
    match action {
        StoreAction::Erase { db, address, address_stdin, default_country_code, drop_ids, compact, key } => {
            let address = match (address, address_stdin) {
                (Some(a), false) => a,
                (None, true) => {
                    let mut line = String::new();
                    io::stdin().read_line(&mut line).map_err(|e| format!("cannot read the address: {e}"))?;
                    line
                }
                _ => return Err("give the person with --address or --address-stdin".to_owned()),
            };
            if !db.exists() {
                return Err(format!("{} does not exist", db.display()));
            }
            let mut opts = EraseOptions::default().with_drop_ids(drop_ids);
            if let Some(code) = default_country_code {
                opts = opts.with_default_country_code(code);
            }
            warn_if_synced(&db);
            let mut options = StoreOptions::default();
            if let Some(key) = key.read()? {
                options = options.with_key(key);
            }
            let mut store = SqliteStore::open_with(&db, &options).map_err(|e| e.to_string())?;
            let erased = store.erase_person(&address, &opts).map_err(|e| e.to_string())?;
            if compact {
                store.compact().map_err(|e| e.to_string())?;
            }
            Ok(format!("erased {erased} event(s)"))
        }
        StoreAction::Encrypt { db, output, keep_plain, key } => {
            warn_if_synced(&output);
            let key = key.read()?.ok_or_else(|| "give the key with --key-file or --key-stdin".to_owned())?;
            let report = encrypt_database(&db, &output, &key, keep_plain).map_err(|e| e.to_string())?;
            let tail = if report.plain_deleted {
                "the plain file was overwritten and deleted"
            } else {
                "the plain file was kept: delete it yourself once you rely on the encrypted one"
            };
            Ok(format!(
                "encrypted {} row(s) into the new file, verified; {tail}. Back up the key separately from the database",
                report.rows
            ))
        }
    }
}

fn read_input(path: &Path) -> io::Result<String> {
    if path.as_os_str() == "-" {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        fs::read_to_string(path)
    }
}

/// The limits the flags ask for; flags that are not given keep the defaults.
fn limits_of(cli: &Cli) -> Limits {
    if cli.no_limits {
        return Limits::unlimited();
    }
    let mut limits = Limits::default();
    limits.max_input_bytes = cli.max_input_bytes;
    limits.max_line_bytes = cli.max_line_bytes;
    limits.max_messages = cli.max_messages;
    limits.max_json_depth = cli.max_json_depth;
    limits
}

fn run(cli: Cli) -> Result<ImportReport, String> {
    let limits = limits_of(&cli);
    let read = |p: &Path| read_input(p).map_err(|e| format!("cannot read {}: {e}", p.display()));
    match cli.command {
        Command::WhatsappExport {
            file,
            self_name,
            date_order,
            utc_offset_minutes,
            conversation_id,
            default_country_code,
            time_zone,
        } => {
            let mut opts = WhatsAppExportOptions::default()
                .with_date_order(date_order.into())
                .with_utc_offset_minutes(utc_offset_minutes);
            opts.self_name = self_name;
            opts.conversation_id = conversation_id;
            opts.default_country_code = default_country_code;
            opts.time_zone = time_zone;
            opts.limits = limits;
            whatsapp_export::try_parse(&read(&file)?, &opts).map_err(|e| e.to_string())
        }
        Command::FacebookExport { files, self_name } => {
            let mut opts = FacebookExportOptions::default();
            opts.self_name = self_name;
            opts.limits = limits;
            let mut merged = ImportReport::default();
            for f in files {
                let r = facebook::parse(&read(&f)?, &opts).map_err(|e| format!("{}: {e}", f.display()))?;
                merged.events.extend(r.events);
                merged.warnings.extend(r.warnings.into_iter().map(|w| format!("{}: {w}", f.display())));
            }
            merged.events.sort_by_key(msgkit_core::Event::timestamp);
            Ok(merged)
        }
        Command::Device { file, own_number, default_country_code } => {
            let batch: DeviceBatch =
                serde_json::from_str(&read(&file)?).map_err(|e| format!("invalid device batch: {e}"))?;
            let mut opts = DeviceOptions::default();
            opts.own_number = own_number;
            opts.default_country_code = default_country_code;
            opts.limits = limits;
            device::try_normalize(&batch, &opts).map_err(|e| e.to_string())
        }
        Command::Store { .. } | Command::Version => unreachable!("handled in main"),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Command::Version) {
        println!("msgkit {} (schema {SCHEMA_VERSION})", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if let Command::Store { action } = cli.command {
        return match run_store(action) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let strict = cli.fail_on_warnings;

    let report = match run(cli) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    if let Err(e) = write_ndjson(&mut out, &report.events).and_then(|()| out.flush()) {
        // A closed pipe (e.g. `| head`) is not an error worth reporting.
        if e.kind() != io::ErrorKind::BrokenPipe {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    }
    if strict && !report.warnings.is_empty() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
