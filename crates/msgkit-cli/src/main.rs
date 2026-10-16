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
use msgkit_core::store::write_ndjson;
use msgkit_core::whatsapp_export::{self, DateOrder, WhatsAppExportOptions};
use msgkit_core::{ImportReport, SCHEMA_VERSION};

#[derive(Parser)]
#[command(name = "msgkit", version, about = "Convert message exports to normalized NDJSON")]
struct Cli {
    /// Exit with an error if the import produced any warnings.
    #[arg(long, global = true)]
    fail_on_warnings: bool,

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
    /// Print version information.
    Version,
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

fn run(cli: Cli) -> Result<ImportReport, String> {
    let read = |p: &Path| read_input(p).map_err(|e| format!("cannot read {}: {e}", p.display()));
    match cli.command {
        Command::WhatsappExport {
            file,
            self_name,
            date_order,
            utc_offset_minutes,
            conversation_id,
            default_country_code,
        } => {
            let opts = WhatsAppExportOptions {
                self_name,
                date_order: date_order.into(),
                utc_offset_minutes,
                conversation_id,
                default_country_code,
            };
            Ok(whatsapp_export::parse(&read(&file)?, &opts))
        }
        Command::FacebookExport { files, self_name } => {
            let opts = FacebookExportOptions { self_name, ..Default::default() };
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
            Ok(device::normalize(&batch, &DeviceOptions { own_number, default_country_code }))
        }
        Command::Version => unreachable!("handled in main"),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Command::Version) {
        println!("msgkit {} (schema {SCHEMA_VERSION})", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
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
