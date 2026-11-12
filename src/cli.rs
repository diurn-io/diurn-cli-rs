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

    /// Market calendars and trading hours, from the Diurn API. Requires an API key.
    #[command(
        subcommand,
        long_about = "Market calendars and trading hours, from the Diurn API.\n\n\
            Calendar commands require DIURN_API_KEY: a free key is at \
            https://diurn.io/signin/. Every command is keyed by MIC and is a \
            thin wrapper over the `diurn` client crate, so it answers exactly \
            what a program using the crate would.\n\n\
            Dates are YYYY-MM-DD; instants are RFC 3339 with an offset or Z \
            (2026-11-27T15:00:00Z), or `now`. A day Diurn cannot answer is \
            never reported as open or closed: the command exits 5.",
        after_help = "EXIT CODES:\n  \
            0  answered\n  \
            1  usage: a malformed argument or request\n  \
            3  the API could not be reached\n  \
            4  the API refused: no or inactive key, not in your plan, outside \
            its dates, a MIC cap, the rate, or an unknown MIC\n  \
            5  indeterminate: Diurn cannot say whether the market trades then\n\n\
            MIC commands need no key and work offline: try `diurn mic --help`."
    )]
    Cal(CalCommand),
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

#[derive(Debug, Subcommand)]
pub enum CalCommand {
    /// Whether venues are trading at an instant, and their next open and close.
    Status {
        /// One or more MICs, e.g. XNYS XLON.
        #[arg(required = true, value_name = "MIC")]
        mics: Vec<String>,
        /// The instant to ask about; now by default.
        #[arg(long, value_name = "INSTANT")]
        at: Option<String>,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// The next instant a venue starts trading.
    NextOpen {
        /// The MIC, e.g. XNYS.
        mic: String,
        /// Look after this instant; now by default.
        #[arg(long, value_name = "INSTANT")]
        after: Option<String>,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// The next instant a venue stops trading.
    NextClose {
        /// The MIC, e.g. XNYS.
        mic: String,
        /// Look after this instant; now by default.
        #[arg(long, value_name = "INSTANT")]
        after: Option<String>,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// Every day in a range for a venue, with its sessions.
    Sessions {
        /// The MIC, e.g. XNYS.
        mic: String,
        #[command(flatten)]
        range: RangeArgs,
        /// Only days of these kinds, e.g. holiday,early_close.
        #[arg(long, value_delimiter = ',', value_name = "KIND")]
        kinds: Vec<String>,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// One day for a venue, with the evidence behind it.
    Day {
        /// The MIC, e.g. XNYS.
        mic: String,
        /// The date, YYYY-MM-DD.
        date: String,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// The nth trading day after a date: for one venue, or the next date
    /// several trade (all of them, or any one with --mode any).
    Next(StepArgs),

    /// The nth trading day before a date, as `next` counts forward.
    Previous(StepArgs),

    /// Trading days in a range, inclusive.
    Count {
        /// The MIC, e.g. XNYS.
        mic: String,
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// Holidays and irregular days across venues, one row each.
    Exceptions {
        /// One or more MICs, e.g. XNYS XLON XTKS.
        #[arg(required = true, value_name = "MIC")]
        mics: Vec<String>,
        #[command(flatten)]
        range: RangeArgs,
        /// Only these kinds; holiday, early_close, open_for_some and unknown
        /// by default.
        #[arg(long, value_delimiter = ',', value_name = "KIND")]
        kinds: Vec<String>,
        #[command(flatten)]
        as_of: AsOfArg,
    },

    /// Your plan, and its meters: unique MICs, dates and rate. Never metered.
    Usage,
}

/// A date range. Both ends are inclusive.
#[derive(Debug, Args)]
pub struct RangeArgs {
    /// The first date, YYYY-MM-DD; today (UTC) by default.
    #[arg(long, value_name = "DATE")]
    pub from: Option<String>,
    /// The last date, YYYY-MM-DD; 30 days after --from by default.
    #[arg(long, value_name = "DATE")]
    pub to: Option<String>,
}

#[derive(Debug, Args)]
pub struct StepArgs {
    /// One or more MICs. With several, the date they trade together.
    #[arg(required = true, value_name = "MIC")]
    pub mics: Vec<String>,
    /// Start from this date, YYYY-MM-DD; today (UTC) by default.
    #[arg(long, value_name = "DATE")]
    pub from: Option<String>,
    /// Trading days to move.
    #[arg(short, long, default_value_t = 1)]
    pub n: u32,
    /// Count the starting date itself when it trades.
    #[arg(long)]
    pub inclusive: bool,
    /// With several MICs: every venue trading (all) or at least one (any).
    #[arg(long, value_enum, default_value_t = Mode::All)]
    pub mode: Mode,
    #[command(flatten)]
    pub as_of: AsOfArg,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    All,
    Any,
}

/// Answer as Diurn would have at a past instant (Enterprise).
#[derive(Debug, Args)]
pub struct AsOfArg {
    /// Answer from the dataset Diurn was serving at this instant (Enterprise).
    #[arg(long, value_name = "INSTANT")]
    pub as_of: Option<String>,
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
