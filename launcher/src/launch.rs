use std::collections::HashMap;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use regex::Regex;

use crate::config::Config;
use crate::local_mods;

const SERVER_ROOT: &str = "/arma3/server";
const TMP_CONFIG: &str = "/tmp/arma3.cfg";

/// `sh -c <cmd>` with SIGPIPE and SIGXFSZ restored to SIG_IGN in the child,
/// matching the signal dispositions v2's server actually ran under.
///
/// v2 launched the server with `os.system()`, which fork/execs without
/// touching signal dispositions, so arma3server_x64 inherited CPython's
/// own SIG_IGN for SIGPIPE and a write to a closed pipe merely returned
/// EPIPE. Rust's `Command` deliberately resets SIGPIPE to SIG_DFL in the
/// child (libstd ignores it process-wide, and it resets so the spawned
/// program starts from a standard state), so the exact same server binary
/// gets *killed* by SIGPIPE instead -- exiting 141 (128+13) and taking the
/// whole server down. Reported against v3 by an extension that loads a JVM,
/// which is the kind of thing that closes a pipe under a foreign thread;
/// the extension is not doing anything wrong, v2 just silently tolerated
/// this and v3 did not.
///
/// SIGXFSZ is the other signal CPython ignores process-wide (confirmed by
/// decoding a v2 child's `SigIgn` mask: bits 13 and 25, nothing else). It
/// only fires when RLIMIT_FSIZE is set, which it is not by default -- but
/// where it does fire, v2 turned an oversized write (a large .rpt, say)
/// into an EFBIG the server could handle, and a SIG_DFL v3 would instead
/// kill it. Restored here too so the set matches v2 exactly rather than
/// leaving one arbitrary difference behind.
fn sh_command(cmd: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(cmd);
    // SAFETY: runs in the child between fork and exec, where only
    // async-signal-safe calls are permitted. `signal(2)` is on POSIX's
    // async-signal-safe list, and nothing here allocates or takes a lock.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            Ok(())
        });
    }
    command
}

fn mod_param(name: &str, mods: &[String]) -> String {
    if mods.is_empty() {
        return String::new();
    }
    format!(" -{}=\"{}\" ", name, mods.join(";"))
}

fn env_defined(key: &str) -> bool {
    std::env::var(key).map(|v| !v.is_empty()).unwrap_or(false)
}

/// Parse an Arma config's `key = value` lines the same loose way the old
/// Python regex did, keyed lowercase for case-insensitive lookups.
fn parse_config_values(data: &str) -> HashMap<String, String> {
    // Mirrors the original: r"(.+?)(?:\s+)?=(?:\s+)?(.+?)(?:$|\/|;)" with MULTILINE.
    let re = Regex::new(r"(?m)(.+?)(?:\s+)?=(?:\s+)?(.+?)(?:$|/|;)").unwrap();
    re.captures_iter(data)
        .map(|c| (c[1].trim().to_lowercase(), c[2].trim().to_string()))
        .collect()
}

/// Build the launch command, spawn headless clients, and exec the server —
/// the Rust port of `launch.py`. `process_start`/`authed_at`/`resolved_at`
/// are captured in `main()` (process start, right after acquiring a CM
/// connection, and right after server depot resolution finishes) so the
/// final log can break out login time, depot resolution time, and
/// download/verify time separately, plus the total.
pub async fn run(
    cfg: &Config,
    mods: Vec<String>,
    process_start: std::time::Instant,
    authed_at: std::time::Instant,
    resolved_at: std::time::Instant,
) -> Result<()> {
    // Clean up previous HC command scripts.
    for entry in glob_hc_scripts()? {
        std::fs::remove_file(&entry)
            .with_context(|| format!("failed to remove old HC script {}", entry.display()))?;
        tracing::info!("Removed old HC script: {}", entry.display());
    }

    let mut launch = format!(
        "{} -limitFPS={} -world={} {} {}",
        cfg.arma_binary,
        std::env::var("ARMA_LIMITFPS").unwrap_or_else(|_| "1000".into()),
        std::env::var("ARMA_WORLD").unwrap_or_else(|_| "empty".into()),
        std::env::var("ARMA_PARAMS").unwrap_or_default(),
        mod_param("mod", &mods),
    );

    for cdlc in &cfg.arma_cdlc {
        launch.push_str(&format!(" -mod={cdlc}"));
    }

    let clients: u32 = std::env::var("HEADLESS_CLIENTS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .context("HEADLESS_CLIENTS must be an integer")?;
    tracing::info!("Headless Clients: {clients}");

    println!(
        "\n==========================================\n=                                        =\n=   IT'S LAUNCHING, HAIL TO THE KING     =\n=                                        =\n==========================================\n"
    );
    let now = std::time::Instant::now();
    tracing::info!(
        "Ready in {:.1}s total (login {:.1}s, depot resolution {:.1}s, download/verify {:.1}s)",
        (now - process_start).as_secs_f64(),
        (authed_at - process_start).as_secs_f64(),
        (resolved_at - authed_at).as_secs_f64(),
        (now - resolved_at).as_secs_f64(),
    );

    std::env::set_current_dir(SERVER_ROOT)
        .with_context(|| format!("failed to chdir into {SERVER_ROOT}"))?;

    let config_file = std::env::var("ARMA_CONFIG").context("missing ARMA_CONFIG")?;
    let port = std::env::var("PORT").unwrap_or_else(|_| "2302".into());

    if clients != 0 {
        let config_path = format!("{SERVER_ROOT}/configs/{config_file}");
        let mut data = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {config_path}"))?;

        let config_values = parse_config_values(&data);

        if !config_values.contains_key("headlessclients[]") {
            data.push_str("\nheadlessclients[] = {\"127.0.0.1\"};\n");
        }
        if !config_values.contains_key("localclient[]") {
            data.push_str("\nlocalclient[] = {\"127.0.0.1\"};\n");
        }

        std::fs::write(TMP_CONFIG, &data).context("failed to write temp config")?;
        launch.push_str(&format!(" -config=\"{TMP_CONFIG}\""));

        let mut client_launch = launch.clone();
        client_launch.push_str(&format!(" -client -connect=127.0.0.1 -port={port}"));
        if let Some(password) = config_values.get("password") {
            client_launch.push_str(&format!(" -password={password}"));
        }

        let hc_profile_template =
            std::env::var("HEADLESS_CLIENTS_PROFILE").unwrap_or_else(|_| "$profile-hc-$i".into());
        let arma_profile = std::env::var("ARMA_PROFILE").unwrap_or_else(|_| "main".into());

        for i in 0..clients {
            // Substitute $ii before $i, since $i is a prefix of $ii.
            let hc_name = hc_profile_template
                .replace("$profile", &arma_profile)
                .replace("$ii", &(i + 1).to_string())
                .replace("$i", &i.to_string());

            let hc_launch = format!("{client_launch} -name=\"{hc_name}\"");

            let hc_script_path = format!("{SERVER_ROOT}/hc_command_{}.sh", i + 1);
            std::fs::write(
                &hc_script_path,
                format!("#!/bin/bash\ncd {SERVER_ROOT}\n{hc_launch}\n"),
            )
            .with_context(|| format!("failed to write {hc_script_path}"))?;
            std::fs::set_permissions(
                &hc_script_path,
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )?;
            tracing::info!("Saved HC command to: {hc_script_path}");

            tracing::info!("LAUNCHING ARMA CLIENT {i} WITH {hc_launch}");
            // Same SIGPIPE treatment as the server below: HCs run the same
            // binary with the same mods loaded, so an extension that kills
            // the server this way kills an HC too. (v2 spawned HCs via
            // subprocess.Popen, whose restore_signals=True default *did*
            // reset SIGPIPE to SIG_DFL -- so this is deliberately not
            // bug-for-bug with v2, which was inconsistent between the two.)
            sh_command(&hc_launch)
                .spawn()
                .with_context(|| format!("failed to spawn headless client {i}"))?;
        }
    }

    // INIT_MISSION: auto-start a mission on server launch.
    if env_defined("INIT_MISSION") {
        let mut init_mission = std::env::var("INIT_MISSION").unwrap();
        if let Some(stripped) = init_mission.strip_suffix(".pbo") {
            init_mission = stripped.to_string();
        }
        tracing::info!("INIT_MISSION set to: {init_mission}");

        let mut data = if launch.contains(&format!("-config=\"{TMP_CONFIG}\"")) {
            std::fs::read_to_string(TMP_CONFIG)?
        } else {
            std::fs::read_to_string(format!("{SERVER_ROOT}/configs/{config_file}"))?
        };

        if !data.to_lowercase().contains("persistent") {
            data.push_str("\npersistent = 1;\n");
        }
        data.push_str(&format!(
            "\nclass Missions {{\n  class Mission_1 {{\n    template = \"{init_mission}\";\n    difficulty = \"skua_difficulty\";\n  }};\n}};\n"
        ));

        std::fs::write(TMP_CONFIG, data).context("failed to write temp config for INIT_MISSION")?;
        launch.push_str(" -autoInit");
    }

    if !launch.contains(&format!("-config=\"{TMP_CONFIG}\"")) {
        launch.push_str(&format!(" -config=\"{SERVER_ROOT}/configs/{config_file}\""));
    }

    let arma_profile = std::env::var("ARMA_PROFILE").unwrap_or_else(|_| "main".into());
    let network_config = std::env::var("NETWORK_CONFIG").context("missing NETWORK_CONFIG")?;
    launch.push_str(&format!(
        " -port={port} -name=\"{arma_profile}\" -profiles=\"{SERVER_ROOT}/configs/profiles\" -cfg=\"{SERVER_ROOT}/configs/{network_config}\""
    ));

    let servermods_dir = Path::new("servermods");
    if servermods_dir.exists() {
        let server_mods = local_mods::mods(servermods_dir)?;
        launch.push_str(&mod_param("serverMod", &server_mods));
    }

    tracing::info!("LAUNCHING ARMA SERVER WITH {launch}");
    let status = sh_command(&launch)
        .status()
        .context("failed to exec arma3server")?;

    if !status.success() {
        anyhow::bail!(
            "arma3server exited with status {}",
            status
                .code()
                .unwrap_or_else(|| status.signal().unwrap_or(-1))
        );
    }

    Ok(())
}

fn glob_hc_scripts() -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    let dir = Path::new(SERVER_ROOT);
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("hc_command_") && name.ends_with(".sh") {
            out.push(entry.path());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn sh_command_child_matches_v2_sigign_mask() {
        let out = super::sh_command("grep '^SigIgn' /proc/self/status")
            .output()
            .unwrap();
        let mask = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();
        let bits = u64::from_str_radix(&mask, 16).unwrap();
        // 13 = SIGPIPE, 25 = SIGXFSZ -- exactly what CPython ignored in v2.
        assert_eq!(bits & (1 << 12), 1 << 12, "SIGPIPE not ignored (mask {mask})");
        assert_eq!(bits & (1 << 24), 1 << 24, "SIGXFSZ not ignored (mask {mask})");
    }
}
