use std::env::{self, current_exe};

use clap::Parser;

mod command;
mod helpers;
use command::Commands;

mod config;
mod record;


#[derive(Parser)]
#[command(
    author = "De-Great Yartey <mail@degreat.co.uk>",
    version = "0.0.2",
    about = "Run .local DNS resolution for apps in development",
    long_about = "Run .local DNS resolution for apps in development"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

fn main() {
    let exe_path = current_exe().unwrap();
    let exe_dir = exe_path.parent().unwrap();
    env::set_current_dir(exe_dir).expect("failed to run command from its directory");

    let cli = Cli::parse();

    let command = match &cli.command {
        Some(cmd) => cmd,
        None => {
            return;
        }
    };

    command.exec();
}
