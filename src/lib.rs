//! notify-gate (`ngate`): tell recipients about new lmm messages through
//! adapters. See `docs/prd.md` for the specification.

pub mod adapter;
pub mod cli;
pub mod config;
pub mod error;
pub mod guard;
pub mod home;
pub mod link;
pub mod lmm;
pub mod template;
pub mod util;

use clap::Parser;

use crate::cli::{Cli, Cmd};

/// Run the command line and return the exit code.
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            let _ = e.print();
            return if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                0
            } else {
                2
            };
        }
    };
    let result = match cli.cmd {
        Cmd::Link(l) => link::run(link::LinkArgs {
            address: l.address,
            destination: l.destination,
            detach: l.detach,
            template: l.template,
        }),
        Cmd::Stop(s) => link::stop(&s.address),
        Cmd::List => link::list(),
        Cmd::Guard(g) => match guard::run(g.address.as_deref(), g.template.as_deref()) {
            Ok(code) => return code,
            Err(e) => Err(e),
        },
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ngate: {e}");
            e.code()
        }
    }
}
