use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Market Identifier Codes and trading calendars.
///
/// MIC subcommands work entirely offline and need no API key — the source is a
/// free public file. Calendar subcommands require a key from diurn.io.
#[derive(Debug, Parser)]
#[command(
    name = "diurn",
    version,
    about,
    long_about = None,
    // Subcommand-less invocation should show help, not an opaque error.
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Output format. Defaults to `table` on a terminal, `jsonl` when piped.
    #[arg(long, short, global = true, value_enum)]
    pub format: Option<Format>,

    /// Suppress the vintage banner and other notes on stderr.
    #[arg(long, short, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// ISO 10383 Market Identifier Codes. Offline, no API key.
    #[command(
        subcommand,
        long_about = "ISO 10383 Market Identifier Codes. No API key, and no \
            network except for `fetch`.\n\n\
            Nothing is bundled with this command. Run `diurn mic fetch` once; \
            after that every command uses the newest registry in your data \
            directory. Point at a specific file with --path, and see what you \
            have with `diurn mic vintages`."
    )]
    Mic(MicCommand),

    /// Market calendars and trading hours. Requires an API key.
    #[command(
        long_about = "Market calendars and trading hours.\n\n\
            Calendar commands require DIURN_API_KEY — get one at https://diurn.io\n\n\
            The namespace is reserved but not yet implemented. When it lands it \
            will be a thin wrapper over the `diurn` client crate, so the CLI \
            exercises the same library everyone else integrates against.",
        after_help = "PLANNED COMMANDS:\n  \
            diurn cal status <id>       open or closed at an instant\n  \
            diurn cal next-close <id>   next session close\n  \
            diurn cal next-open <id>    next session open\n  \
            diurn cal sessions <id>     sessions over a date range\n  \
            diurn cal coverage          which calendars are verified how far\n\n\
            MIC commands need no key and work offline: try `diurn mic --help`."
    )]
    Cal(CalArgs),
}

#[derive(Debug, Subcommand)]
pub enum MicCommand {
    /// Parse a registry file and summarise what was found.
    Load {
        /// Path to an ISO 10383 CSV.
        path: PathBuf,
        #[command(flatten)]
        vintage: VintageArgs,
        /// Show every issue rather than a count per kind.
        #[arg(long)]
        issues: bool,
    },

    /// Show a single MIC.
    Get {
        /// The four-character code, e.g. XNYS.
        mic: String,
        /// Also list the segments operating under it.
        #[arg(long)]
        segments: bool,
        #[command(flatten)]
        source: SourceArgs,
    },

    /// List MICs, optionally filtered.
    List {
        /// ISO 3166-1 alpha-2 country code, e.g. US.
        #[arg(long)]
        country: Option<String>,
        /// active, updated, or expired.
        #[arg(long)]
        status: Option<String>,
        /// Market category code, e.g. RMKT. Also accepts the full name.
        #[arg(long)]
        category: Option<String>,
        /// Only operating MICs, excluding segments.
        #[arg(long)]
        operating: bool,
        /// Only records whose changes are not yet in force.
        #[arg(long)]
        pending: bool,
        /// Cap the number of rows.
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        source: SourceArgs,
    },

    /// List the segments operating under a MIC.
    Segments {
        /// The operating MIC, e.g. XNYS.
        mic: String,
        #[command(flatten)]
        source: SourceArgs,
    },

    /// Check a registry file and report every problem found.
    Validate {
        /// Path to an ISO 10383 CSV.
        path: PathBuf,
        #[command(flatten)]
        vintage: VintageArgs,
    },

    /// Compare two vintages.
    Diff {
        /// The earlier file.
        old: PathBuf,
        /// The later file.
        new: PathBuf,
    },

    /// Download the current registry from ISO. The only command that uses the
    /// network.
    Fetch {
        /// Where to write it. Defaults to the data directory, named by
        /// publication date, so later commands find it on their own.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Override the publication date rather than deriving it.
        #[arg(long)]
        published: Option<String>,
    },

    /// List the registry files available locally.
    ///
    /// Commands that are not given `--path` use the first one listed.
    Vintages,
}

/// Which registry file a read-only command should use.
///
/// Flattened into each command, so it contributes the flags `--path` and
/// `--published` directly — the field name it is bound to is invisible on the
/// command line.
#[derive(Debug, Args)]
pub struct SourceArgs {
    /// Registry file to read.
    ///
    /// Defaults to the newest one in the data directory — see
    /// `diurn mic vintages`. Nothing is bundled with this command, so there is
    /// always a real file behind every answer.
    #[arg(long, short = 'p', value_name = "FILE")]
    pub path: Option<PathBuf>,

    /// Publication date of `--path`, if it cannot be read from the filename.
    #[arg(long, requires = "path")]
    pub published: Option<String>,
}

/// Vintage identification for commands that take a path positionally.
#[derive(Debug, Args)]
pub struct VintageArgs {
    /// Publication date, if it cannot be read from the filename.
    #[arg(long)]
    pub published: Option<String>,
}

#[derive(Debug, Args)]
pub struct CalArgs {
    /// Swallows whatever follows so that `diurn cal status XNYS` reports the
    /// missing key rather than an unknown-subcommand error. Hidden: it is a
    /// placeholder, not an argument anyone should be told about.
    #[arg(hide = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Aligned columns for reading.
    Table,
    /// One JSON document.
    Json,
    /// One JSON object per line, for streaming into other tools.
    Jsonl,
    /// Comma-separated, with a header row.
    Csv,
}
