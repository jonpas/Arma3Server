use std::collections::HashMap;
use std::env;

use once_cell::sync::Lazy;

/// depot IDs for each free Creator DLC, as they appear on app 233780's
/// `creatordlc` branch. Mirrors the old `api.py::CDLC_IDS` mapping.
static CDLC_DEPOT_IDS: Lazy<HashMap<&'static str, u32>> = Lazy::new(|| {
    HashMap::from([
        ("csla", 233793),
        ("gm", 233792),
        ("vn", 233794),
        ("ws", 233795),
        ("spe", 233788),
        ("rf", 233799),
        ("ef", 233798),
    ])
});

/// Steam login mode: either an anonymous session (works for the base
/// server + Linux binary depots, but not CDLC or workshop content, since
/// Steam won't grant free licenses or item access to anonymous sessions),
/// or a real account.
#[derive(Clone)]
pub enum SteamAuth {
    Anonymous,
    Credentials { user: String, password: String },
}

pub struct Config {
    pub steam_auth: SteamAuth,
    pub arma_binary: String,
    pub arma_cdlc: Vec<String>,
    pub mods_preset: Option<String>,
    pub mods_local: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let anonymous_login = bool_env("ANONYMOUS_LOGIN", false);
        let steam_auth = if anonymous_login {
            SteamAuth::Anonymous
        } else {
            SteamAuth::Credentials {
                user: require_env("STEAM_USER")?,
                password: require_env("STEAM_PASSWORD")?,
            }
        };

        Ok(Self {
            steam_auth,
            arma_binary: env::var("ARMA_BINARY").unwrap_or_else(|_| "./arma3server_x64".into()),
            arma_cdlc: env::var("ARMA_CDLC")
                .unwrap_or_default()
                .split(';')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
            mods_preset: env::var("MODS_PRESET").ok().filter(|s| !s.is_empty()),
            mods_local: bool_env("MODS_LOCAL", true),
        })
    }

    pub fn wants_profiling(&self) -> bool {
        self.arma_binary == "arma3serverprofiling_x64"
    }

    /// Resolve configured CDLC names to their depot IDs, warning on any
    /// name that isn't recognized instead of failing the whole install.
    pub fn cdlc_depot_ids(&self) -> Vec<u32> {
        self.arma_cdlc
            .iter()
            .filter_map(|name| match CDLC_DEPOT_IDS.get(name.as_str()) {
                Some(id) => Some(*id),
                None => {
                    tracing::warn!("unknown CDLC '{name}', skipping");
                    None
                }
            })
            .collect()
    }
}

fn require_env(key: &str) -> anyhow::Result<String> {
    env::var(key).map_err(|_| anyhow::anyhow!("missing required env var {key}"))
}

fn bool_env(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => v.eq_ignore_ascii_case("true"),
        Err(_) => default,
    }
}
