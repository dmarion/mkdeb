use clap::{ArgAction, ColorChoice, Parser};

/// mkdeb: GitHub → build → .deb
#[derive(Parser, Debug)]
#[command(
    name = "mkdeb",
    version,
    about = "Build and package GitHub-hosted projects into .deb files",
    color = ColorChoice::Always
)]
pub struct CliArgs {
    /// Enable verbose output
    #[arg(short, long, action = ArgAction::Count, help = "Increase verbosity (-v, -vv)")]
    pub verbose: u8,

    /// Enable debug output
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub debug: bool,

    /// Path to configuration TOML file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Install the built package unless same version is installed
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub install: bool,

    /// List installed and configured versions
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub list: bool,

    /// Operate on all packages
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub all: bool,

    /// Comma-separated list of packages to operate on
    #[arg(short = 'p')]
    pub packages: Option<String>,

    /// Enable per-step logging
    #[arg(long, action = ArgAction::SetTrue)]
    pub log: bool,

    /// Directory where logs will be written
    #[arg(long = "log-dir")]
    pub log_dir: Option<String>,

    /// Use specified path instead of a temporary directory for building
    #[arg(long)]
    pub build_root: Option<String>,
}
