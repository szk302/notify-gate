//! Command line (clap).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "ngate",
    version,
    about = "Tell recipients about new lmm messages through adapters"
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Tell a recipient about new messages through a destination, until the
    /// destination is gone
    Link(LinkCli),
    /// Stop the link of an address
    Stop(StopCli),
    /// List the links
    List,
    /// Exit 2 with the reason if extracted messages are left (for tmac's
    /// relay of Stop)
    Guard(GuardCli),
}

#[derive(Args, Debug)]
pub struct LinkCli {
    /// Start in the background and return at once
    #[arg(long)]
    pub detach: bool,
    /// YAML file with the texts of the notices (new, reminder)
    #[arg(long, value_name = "FILE")]
    pub template: Option<PathBuf>,
    /// The recipient (an lmm address)
    pub address: String,
    /// tmac (the agent of the relay), tmac:PROJECT/NAME, or a name in
    /// destinations of the configuration
    pub destination: String,
}

#[derive(Args, Debug)]
pub struct StopCli {
    pub address: String,
}

#[derive(Args, Debug)]
pub struct GuardCli {
    /// YAML file with the text of the reason (guard)
    #[arg(long, value_name = "FILE")]
    pub template: Option<PathBuf>,
    /// The recipient (default: your lmm address here)
    pub address: Option<String>,
}
