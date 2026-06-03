use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeagueInstallation {
    pub root: PathBuf,
    pub game_executable: PathBuf,
    pub source: String,
}

pub fn detect_league_installations() -> Vec<LeagueInstallation> {
    let mut candidates = Vec::new();

    if let Ok(riot_games) = env::var("RIOT_GAMES_PATH") {
        candidates.push((PathBuf::from(riot_games).join("League of Legends"), "RIOT_GAMES_PATH"));
    }

    for env_key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(program_files) = env::var(env_key) {
            candidates.push((
                PathBuf::from(program_files)
                    .join("Riot Games")
                    .join("League of Legends"),
                env_key,
            ));
        }
    }

    candidates
        .into_iter()
        .filter_map(|(root, source)| {
            let exe = root.join("LeagueClient.exe");
            if exe.exists() {
                Some(LeagueInstallation {
                    root,
                    game_executable: exe,
                    source: source.to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}
