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
        manifest: PathBuf,
        /// Required for a dry run; ignored when --out is given.
        #[arg(long)]
        league_root: Option<PathBuf>,
        /// Build overlay WADs into this directory instead of a dry run.
        #[arg(long)]
        out: Option<PathBuf>,
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
            manifest,
            league_root,
            out,
        } => {
            let manifest_path = manifest;
            let manifest = load_manifest(&manifest_path).context("load manifest")?;
            let mut profile = Profile::new("CLI");
            profile.enable_mod(manifest.id);
            let package_path = manifest_path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let library = vec![LibraryItem {
                manifest,
                package_path,
                imported_at: OffsetDateTime::now_utc(),
            }];

            match out {
                Some(out_dir) => {
                    let request = PatchRequest {
                        league_root: league_root
                            .context("--league-root is required when staging with --out")?,
                        dry_run: false,
                        profile,
                        library,
                    };
                    let report = PatchEngine::stage(&request, &out_dir)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                None => {
                    let league_root =
                        league_root.context("--league-root is required for a dry run")?;
                    let report = PatchEngine::plan(PatchRequest {
                        league_root,
                        dry_run: true,
                        profile,
                        library,
                    })?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
            }
        }
        Command::Doctor => {
            let installations = detect_league_installations();
            println!("{}", serde_json::to_string_pretty(&installations)?);
        }
    }

    Ok(())
}
