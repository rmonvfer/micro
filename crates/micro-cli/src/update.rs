//! Installing releases into the managed distribution directory.

use anyhow::Context as _;
use anyhow::Result;
use fs2::FileExt as _;
use futures::TryStreamExt as _;
use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use std::ffi::OsString;
use std::fs::File;
use std::io::IsTerminal as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncWriteExt as _;

const REPOSITORY: &str = "rmonvfer/micro";
const CHECK_INTERVAL_HOURS: u64 = 24;
const STATE_FILE: &str = "update.json";
const LOCK_FILE: &str = "update.lock";
const BINARY: &str = "micro";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Current {
        version: String,
    },
    Installed {
        previous_version: String,
        version: String,
        launcher: PathBuf,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    last_checked_at: u64,
}

struct Installation {
    launcher: PathBuf,
    dist_dir: PathBuf,
    version_dir: PathBuf,
}

struct UpdateLock {
    _file: File,
}

pub async fn automatic(args: &[OsString], enabled: bool, interval_hours: u64) -> Option<PathBuf> {
    if !enabled || !should_check_automatically(args) {
        return None;
    }

    match check_and_install(false, interval_hours).await {
        Ok(Outcome::Installed {
            previous_version,
            version,
            launcher,
        }) => {
            println!("Updated micro {previous_version} to {version}. Restarting...");
            Some(launcher)
        }
        Ok(_) => None,
        Err(error) => {
            eprintln!("note: micro could not check for updates: {error}");
            None
        }
    }
}

pub async fn update_now() -> Result<Outcome> {
    check_and_install(true, CHECK_INTERVAL_HOURS).await
}

pub fn restart(launcher: &Path, args: &[OsString]) -> Result<i32> {
    let status = std::process::Command::new(launcher)
        .args(args)
        .status()
        .with_context(|| format!("could not restart {}", launcher.display()))?;
    Ok(status.code().unwrap_or(1))
}

fn should_check_automatically(args: &[OsString]) -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var("MICRO_NO_AUTO_UPDATE")
            .map(|value| !matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(true)
        && arguments_allow_auto_update(args)
}

fn arguments_allow_auto_update(args: &[OsString]) -> bool {
    !args.iter().skip(1).any(|argument| {
        matches!(
            argument.to_string_lossy().as_ref(),
            "--print" | "-p" | "--rpc" | "--help" | "-h" | "--version" | "update"
        )
    })
}

async fn check_and_install(force: bool, interval_hours: u64) -> Result<Outcome> {
    let installation = match managed_installation() {
        Ok(installation) => installation,
        Err(reason) => return Ok(Outcome::Skipped { reason }),
    };
    let state_path = micro_dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("no data directory; set {}", micro_dirs::MICRO_DIR_ENV))?
        .join(STATE_FILE);
    let lock_path = state_path.with_file_name(LOCK_FILE);
    let _lock = acquire_lock(&lock_path)?;
    if !force && checked_recently(&state_path, interval_hours) {
        return Ok(Outcome::Skipped {
            reason: "an update check is not due yet".to_string(),
        });
    }

    let result = check_and_install_managed(&installation).await;
    if result.is_ok() || !force {
        let _ = record_check(&state_path);
    }
    result
}

async fn check_and_install_managed(installation: &Installation) -> Result<Outcome> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .user_agent(format!("micro/{}", env!("CARGO_PKG_VERSION")))
        .build()?;
    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = Version::parse(release.tag_name.trim_start_matches('v'))
        .with_context(|| format!("release tag {} is not a semantic version", release.tag_name))?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    if version <= current {
        return Ok(Outcome::Current {
            version: current.to_string(),
        });
    }

    let platform = platform().ok_or_else(|| anyhow::anyhow!("unsupported platform"))?;
    let archive_name = format!("micro-{platform}.tar.gz");
    let archive = release
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "release {} does not contain {archive_name}",
                release.tag_name
            )
        })?;
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == "checksums-sha256.txt")
        .ok_or_else(|| anyhow::anyhow!("release {} has no checksums", release.tag_name))?;
    let checksum_text = download_text(&client, &checksums.browser_download_url).await?;
    let expected = checksum_for(&checksum_text, &archive_name)
        .ok_or_else(|| anyhow::anyhow!("release checksums do not contain {archive_name}"))?;

    let staging = tempfile::Builder::new()
        .prefix(".micro-update-")
        .tempdir_in(&installation.dist_dir)?;
    let archive_path = staging.path().join(&archive_name);
    download_file(&client, &archive.browser_download_url, &archive_path).await?;
    let actual = sha256(&archive_path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        anyhow::bail!("checksum verification failed for {archive_name}");
    }

    let extracted = staging.path().join("release");
    std::fs::create_dir(&extracted)?;
    let output = tokio::process::Command::new("tar")
        .args(["-xzf"])
        .arg(&archive_path)
        .args(["-C"])
        .arg(&extracted)
        .args(["--strip-components", "1"])
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "could not extract release: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let staged_binary = extracted.join("bin").join(BINARY);
    if !staged_binary.is_file() {
        anyhow::bail!("release {archive_name} does not contain bin/{BINARY}");
    }

    let destination = installation.dist_dir.join(version.to_string());
    if destination == installation.version_dir {
        anyhow::bail!("release version is already installed");
    }
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&extracted, &destination)?;
    replace_launcher(
        &installation.launcher,
        &destination.join("bin").join(BINARY),
    )?;
    prune_versions(
        &installation.dist_dir,
        &[installation.version_dir.clone(), destination.clone()],
    );

    Ok(Outcome::Installed {
        previous_version: current.to_string(),
        version: version.to_string(),
        launcher: installation.launcher.clone(),
    })
}

async fn download_text(client: &Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

async fn download_file(client: &Client, url: &str, path: &Path) -> Result<()> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await?
        .error_for_status()?;
    let mut file = tokio::fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.try_next().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}

fn managed_installation() -> Result<Installation, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not determine executable: {error}"))?
        .canonicalize()
        .map_err(|error| format!("could not resolve executable: {error}"))?;
    if executable.file_name().is_none_or(|name| name != BINARY) {
        return Err("updates are available only for the packaged micro executable".to_string());
    }
    let bin = executable
        .parent()
        .ok_or_else(|| "could not find executable directory".to_string())?;
    let version_dir = bin
        .parent()
        .ok_or_else(|| "could not find release directory".to_string())?;
    if bin.file_name().is_none_or(|name| name != "bin")
        || version_dir
            .file_name()
            .is_none_or(|name| Version::parse(&name.to_string_lossy()).is_err())
    {
        return Err("updates are available only for a managed micro installation".to_string());
    }
    let dist_dir = version_dir
        .parent()
        .ok_or_else(|| "could not find distribution directory".to_string())?;
    let launcher = find_launcher(&executable).ok_or_else(|| {
        "this installation has no managed launcher symlink; reinstall to enable updates".to_string()
    })?;
    let dist_dir = dist_dir.to_path_buf();
    let version_dir = version_dir.to_path_buf();
    Ok(Installation {
        launcher,
        dist_dir,
        version_dir,
    })
}

fn find_launcher(executable: &Path) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|directory| directory.join(BINARY))
        .find(|candidate| {
            std::fs::symlink_metadata(candidate)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
                && candidate
                    .canonicalize()
                    .is_ok_and(|path| path == executable)
        })
}

fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        _ => None,
    }
}

fn checksum_for<'a>(checksums: &'a str, asset: &str) -> Option<&'a str> {
    checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let checksum = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (parts.next().is_none()
            && name == asset
            && checksum.len() == 64
            && checksum.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(checksum)
    })
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = std::io::BufReader::new(File::open(path)?);
    let mut digest = sha2::Sha256::new();
    std::io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn checked_recently(path: &Path, interval_hours: u64) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(state) = serde_json::from_str::<State>(&text) else {
        return false;
    };
    let now = unix_time();
    now.saturating_sub(state.last_checked_at) < interval_hours.max(1) * 60 * 60
}

fn record_check(path: &Path) -> Result<()> {
    let directory = path.parent().context("update state has no directory")?;
    std::fs::create_dir_all(directory)?;
    let staged = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(
        &staged,
        &State {
            last_checked_at: unix_time(),
        },
    )?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn acquire_lock(path: &Path) -> Result<UpdateLock> {
    let directory = path.parent().context("update lock has no directory")?;
    std::fs::create_dir_all(directory)?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|error| anyhow::anyhow!("could not open the update lock: {error}"))?;
    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("another micro process is checking for updates"))?;
    Ok(UpdateLock { _file: file })
}

fn replace_launcher(launcher: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let directory = launcher.parent().context("launcher has no directory")?;
        let staged = directory.join(format!(".micro-update-{}", unix_time()));
        let _ = std::fs::remove_file(&staged);
        symlink(target, &staged)?;
        std::fs::rename(&staged, launcher)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (launcher, target);
        anyhow::bail!("automatic updates are not supported on this platform")
    }
}

fn prune_versions(dist_dir: &Path, keep: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(dist_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if keep.iter().any(|kept| kept == &path)
            || !path.is_dir()
            || Version::parse(&entry.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        let _ = std::fs::remove_dir_all(path);
    }
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_require_a_complete_matching_record() {
        let sum = "a".repeat(64);
        assert_eq!(
            checksum_for(
                &format!("{sum}  micro-linux-x86_64.tar.gz\n"),
                "micro-linux-x86_64.tar.gz"
            ),
            Some(sum.as_str())
        );
        assert_eq!(
            checksum_for(
                "not-a-checksum  micro-linux-x86_64.tar.gz",
                "micro-linux-x86_64.tar.gz"
            ),
            None
        );
        assert_eq!(
            checksum_for(
                &format!("{sum}  another.tar.gz"),
                "micro-linux-x86_64.tar.gz"
            ),
            None
        );
    }

    #[test]
    fn script_and_update_arguments_do_not_trigger_automatic_checks() {
        assert!(!arguments_allow_auto_update(&[
            OsString::from("micro"),
            OsString::from("--print")
        ]));
        assert!(!arguments_allow_auto_update(&[
            OsString::from("micro"),
            OsString::from("update")
        ]));
        assert!(arguments_allow_auto_update(&[OsString::from("micro")]));
    }

    #[test]
    fn an_update_lock_is_released_for_the_next_process() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update.lock");
        drop(acquire_lock(&path).unwrap());
        assert!(acquire_lock(&path).is_ok());
    }
}
