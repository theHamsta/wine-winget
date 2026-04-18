use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Generate shell completion to stdout
    #[arg(long, value_enum)]
    pub shell_completion: Option<Shell>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Install packages
    Install(Install),

    /// Upgrade packages
    Upgrade(Upgrade),

    /// Remove packages
    Remove(Remove),

    /// Search package
    Search(Search),
}

#[derive(clap::Args, Debug)]
pub struct Install {
    #[arg(required=true,num_args=1..)]
    pub packages: Vec<String>,
    /// Local path to https://github.com/microsoft/winget-pkgs
    #[arg(long, value_enum)]
    pub repo_path: PathBuf,
    /// Version or version requirement
    #[arg(long, value_enum)]
    pub version: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct Remove {
    #[arg(required=true,num_args=1..)]
    pub packages: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct Upgrade {
    #[arg(required=true, num_args=1..)]
    pub packages: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct Search {
    pub search_string: String,
    /// Local path to https://github.com/microsoft/winget-pkgs
    #[arg(long, value_enum)]
    pub repo_path: PathBuf,
}
