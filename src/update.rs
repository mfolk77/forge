use anyhow::{Context, Result};

const REPO_OWNER: &str = "mfolk77";
const REPO_NAME: &str = "forge";

/// Check for a newer release on GitHub. Returns (current, latest, update_available).
pub fn check_for_update() -> Result<(String, String, bool)> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .context("Failed to configure release list")?
        .fetch();

    match releases {
        Ok(list) if !list.is_empty() => {
            let latest = list[0].version.clone();
            let update_available = version_gt(&latest, &current);
            Ok((current, latest, update_available))
        }
        Ok(_) => {
            // No releases published yet
            Ok((current.clone(), current, false))
        }
        Err(_) => {
            // Network error or no releases — treat as up to date
            Ok((current.clone(), current, false))
        }
    }
}

/// Download and install the latest release, replacing the current binary.
pub fn perform_update() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    println!("Current version: v{current}");
    println!("Checking for updates...");

    let status = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name("forge")
        .show_download_progress(true)
        .current_version(current)
        .build()
        .context("Failed to configure updater")?
        .update()
        .context("Failed to perform update")?;

    if status.updated() {
        println!("Updated to v{}!", status.version());
    } else {
        println!("Already on latest version (v{current}).");
    }

    Ok(())
}

/// Simple semver comparison: returns true if a > b.
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let a_part = va.get(i).copied().unwrap_or(0);
        let b_part = vb.get(i).copied().unwrap_or(0);
        if a_part > b_part {
            return true;
        }
        if a_part < b_part {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_gt_major() {
        assert!(version_gt("1.0.0", "0.1.0"));
        assert!(version_gt("2.0.0", "1.9.9"));
    }

    #[test]
    fn test_version_gt_minor() {
        assert!(version_gt("0.2.0", "0.1.0"));
        assert!(version_gt("0.10.0", "0.9.0"));
    }

    #[test]
    fn test_version_gt_patch() {
        assert!(version_gt("0.1.1", "0.1.0"));
    }

    #[test]
    fn test_version_gt_equal() {
        assert!(!version_gt("0.1.0", "0.1.0"));
    }

    #[test]
    fn test_version_gt_less() {
        assert!(!version_gt("0.1.0", "0.2.0"));
    }

    #[test]
    fn test_version_gt_with_v_prefix() {
        assert!(version_gt("v1.0.0", "v0.9.0"));
        assert!(version_gt("v1.0.0", "0.9.0"));
    }

    #[test]
    fn test_version_gt_different_lengths() {
        assert!(version_gt("1.0.1", "1.0"));
        assert!(!version_gt("1.0", "1.0.1"));
    }
}
