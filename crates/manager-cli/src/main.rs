use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use manager_core::{
    assess_manifest_policy, detect_league_installations, load_manifest, LibraryItem, Profile,
};
use manager_patcher::{PatchEngine, PatchRequest};
use std::path::PathBuf;
use time::OffsetDateTime;

#[derive(Debug, Parser)]
#[command(author, version, about = "League mod manager developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Validate { path: PathBuf },
    Import { path: PathBuf },
    Patch {
        #[arg(long)]
        league_root: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
    Doctor,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Validate { path } => {
            let manifest = load_manifest(&path).context("load manifest")?;
            let report = manifest.validate();
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.ok {
                anyhow::bail!("manifest is invalid");
            }
        }
        Command::Import { path } => {
            let manifest = load_manifest(&path).context("load manifest")?;
            let validation = manifest.validate();
            let policy = assess_manifest_policy(&manifest);
            println!("{}", serde_json::to_string_pretty(&(validation, policy))?);
        }
        Command::Patch {
            league_root,
            manifest,
        } => {
            let manifest_path = manifest;
            let manifest = load_manifest(&manifest_path).context("load manifest")?;
            let mut profile = Profile::new("CLI dry-run");
            profile.enable_mod(manifest.id);
            let package_path = manifest_path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let report = PatchEngine::plan(PatchRequest {
                league_root,
                dry_run: true,
                profile,
                library: vec![LibraryItem {
                    manifest,
                    package_path,
                    imported_at: OffsetDateTime::now_utc(),
                }],
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Doctor => {
            let installations = detect_league_installations();
            println!("{}", serde_json::to_string_pretty(&installations)?);
        }
    }

    Ok(())
}
