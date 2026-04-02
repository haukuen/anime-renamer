mod anilist;
mod cli;
mod commands;
mod nfo;
mod parser;
mod scanner;
mod tmdb;

use crate::cli::{Cli, Command, RenameArgs};
use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Nfo(args)) => commands::nfo::run(&args).await,
        None => commands::rename::run(&RenameArgs::try_from(cli.rename)?).await,
    }
}
