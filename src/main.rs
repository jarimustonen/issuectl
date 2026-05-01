use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "issuectl", version, about = "Manage markdown-based issues with frontmatter")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List or query issues by frontmatter
    List,
    /// Detect duplicate issues
    Dedup,
    /// Renumber and fix references during merges
    Renumber,
    /// Generate a Claude Code skill for the current repo
    GenSkill,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => todo!("list"),
        Command::Dedup => todo!("dedup"),
        Command::Renumber => todo!("renumber"),
        Command::GenSkill => todo!("gen-skill"),
    }
}
