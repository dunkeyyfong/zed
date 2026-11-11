//! ZedG fork: self-update from the fork's GitHub Releases instead of zed.dev.
//!
//! The fork's CI publishes every Windows build as a release tagged
//! `zedg-<stamp>`, where `<stamp>` is the UTC build time as `YYYYMMDDHHMM`.
//! Quick test builds are marked as pre-releases. A running build only updates
//! to a release whose stamp is newer than its own, so builds without a stamp
//! (local builds) never update, and never fall back to upstream Zed.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result};
use http_client::{HttpClient, github::latest_github_release};
use semver::{BuildMetadata, Prerelease, Version};
use settings::{RegisterSetting, Settings};
use sha2::{Digest, Sha256};

use crate::{AutoUpdateStatus, ReleaseAsset};

/// The GitHub repository whose releases this build updates from.
const REPO: &str = match option_env!("ZEDG_UPDATE_REPO") {
    Some(repo) => repo,
    None => "dunkeyyfong/zed",
};

/// UTC build time of this binary (`YYYYMMDDHHMM`), injected by the fork's CI.
const BUILD_STAMP: Option<&str> = option_env!("ZEDG_BUILD_STAMP");

const TAG_PREFIX: &str = "zedg-";
const VERSION_BUILD_PREFIX: &str = "zedg.";

/// Test builds land often, so check more frequently than upstream's hourly poll.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Whether auto-update also installs test builds (GitHub pre-releases).
#[derive(Clone, Copy, Debug, RegisterSetting)]
pub(crate) struct TestBuildsSetting(pub bool);

impl Settings for TestBuildsSetting {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self(content.zedg_test_builds.unwrap_or(true))
    }
}

pub(crate) struct Update {
    pub asset: ReleaseAsset,
    pub version: Version,
    pub sha256: Option<String>,
}

pub(crate) fn release_notes_url() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// Returns the newest release that is newer than the running build, if any.
pub(crate) async fn check(
    http: Arc<dyn HttpClient>,
    installed_version: &Version,
    status: &AutoUpdateStatus,
    test_builds: bool,
) -> Result<Option<Update>> {
    let Some(current_stamp) = current_stamp(status) else {
        log::info!("ZedG auto-update: this build has no build stamp, skipping");
        return Ok(None);
    };

    let kinds: &[bool] = if test_builds { &[false, true] } else { &[false] };
    let mut releases = Vec::new();
    let mut last_error = None;
    for &pre_release in kinds {
        match latest_github_release(REPO, true, pre_release, http.clone()).await {
            Ok(release) => releases.push(release),
            Err(error) => last_error = Some(error),
        }
    }
    if releases.is_empty()
        && let Some(error) = last_error
    {
        return Err(error.context(format!("checking {REPO} releases")));
    }

    let Some((stamp, release)) = releases
        .into_iter()
        .filter_map(|release| Some((stamp_from_tag(&release.tag_name)?, release)))
        .max_by_key(|(stamp, _)| *stamp)
    else {
        return Ok(None);
    };
    if stamp <= current_stamp {
        return Ok(None);
    }

    let installer_suffix = format!("{}-setup.exe", std::env::consts::ARCH);
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name.ends_with(&installer_suffix))
        .with_context(|| format!("release {} has no *{installer_suffix}", release.tag_name))?;

    let mut version = installed_version.clone();
    version.pre = Prerelease::EMPTY;
    version.build = BuildMetadata::new(&format!("{VERSION_BUILD_PREFIX}{stamp}"))?;

    Ok(Some(Update {
        asset: ReleaseAsset {
            version: version.to_string(),
            url: asset.browser_download_url,
        },
        version,
        sha256: asset.digest,
    }))
}

/// Checks the downloaded installer against the digest GitHub reports for it.
pub(crate) async fn verify_sha256(path: PathBuf, expected: String) -> Result<()> {
    smol::unblock(move || {
        let actual = sha256_of(&path)?;
        anyhow::ensure!(
            actual.eq_ignore_ascii_case(&expected),
            "update installer SHA-256 mismatch: expected {expected}, got {actual}"
        );
        Ok(())
    })
    .await
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {} to hash it", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn current_stamp(status: &AutoUpdateStatus) -> Option<u64> {
    // After an update is installed but before restarting, compare against the
    // installed update so the same release isn't downloaded again.
    if let AutoUpdateStatus::Updated { version } = status
        && let Some(stamp) = version
            .build
            .as_str()
            .strip_prefix(VERSION_BUILD_PREFIX)
            .and_then(|stamp| stamp.parse().ok())
    {
        return Some(stamp);
    }
    BUILD_STAMP.and_then(|stamp| stamp.trim().parse().ok())
}

fn stamp_from_tag(tag: &str) -> Option<u64> {
    tag.strip_prefix(TAG_PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_tags() {
        assert_eq!(stamp_from_tag("zedg-202611111530"), Some(202611111530));
        assert_eq!(stamp_from_tag("v1.27.0"), None);
        assert_eq!(stamp_from_tag("zedg-abc"), None);
    }

    #[test]
    fn updated_status_overrides_build_stamp() {
        let status = AutoUpdateStatus::Updated {
            version: "1.27.0+zedg.202611111530".parse().unwrap(),
        };
        assert_eq!(current_stamp(&status), Some(202611111530));
    }
}
