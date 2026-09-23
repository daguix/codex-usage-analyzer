mod breakdown;
mod cli;
mod ingest;
mod latency;
mod pricing;
mod report;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    cli::run(cli::Cli::parse())
}
