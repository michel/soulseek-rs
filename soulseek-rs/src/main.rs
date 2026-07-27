//! Entry point. Everything it does lives in the library crate, so the same
//! code is what integration tests drive.

use soulseek_rs_tui::output::{Exit, Out};
use soulseek_rs_tui::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    dotenv::dotenv().ok();
    let _ = color_eyre::install();

    let cli = <soulseek_rs_tui::cli::Cli as clap::Parser>::parse();
    run::init_logging(&cli);

    let out = Out::new(cli.json, cli.quiet);
    match run::run(cli, &out) {
        Ok(()) => Exit::Ok.into(),
        Err(error) => {
            out.error(&error.message);
            error.exit.into()
        }
    }
}
