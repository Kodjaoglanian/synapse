//! Self-update: download and install the latest release from GitHub.
//!
//! Detects the current OS/arch, queries the GitHub releases API for the latest
//! version, downloads the matching asset, and replaces the running binary.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

/// GitHub repository for release assets.
const REPO: &str = "Kodjaoglanian/synapse";

/// Run the update process.
pub fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("synapse v{current} — checking for updates…");

    let target = detect_target()?;
    println!("  platform: {target}");

    let latest = fetch_latest_release()?;
    let latest_version = latest.tag.trim_start_matches('v');
    println!("  latest:   v{latest_version}");

    if latest_version == current {
        println!("You're already up to date.");
        return Ok(());
    }

    let asset_name = format!("synapse-{}-{}.{}", latest.tag, target, archive_ext());
    let asset = latest
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            anyhow!(
                "no asset found for {target} in release {} (looked for {asset_name})",
                latest.tag
            )
        })?;

    println!("  downloading {asset_name}…");
    let data = download_asset(&asset.url)?;
    println!("  downloaded {} bytes", data.len());

    // Write to a temp file, extract, and install.
    let tmp = env::temp_dir().join("synapse-update");
    fs::create_dir_all(&tmp)?;
    let archive_path = tmp.join(&asset_name);
    fs::write(&archive_path, &data)?;

    let extract_dir = tmp.join("extracted");
    fs::create_dir_all(&extract_dir)?;
    extract_archive(&archive_path, &extract_dir, &asset_name)?;

    // Find the synapse binary inside the extracted dir.
    let binary = find_binary(&extract_dir)?;
    install_binary(&binary)?;

    println!("  ✓ updated to v{}", latest.tag);
    println!("  restart synapse to use the new version.");
    Ok(())
}

/// Detect the Rust target triple for the current platform.
fn detect_target() -> Result<String> {
    let arch = env::consts::ARCH;
    let os = env::consts::OS;
    let target = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(anyhow!("unsupported platform: {os}/{arch}")),
    };
    Ok(target.into())
}

/// File extension for the release archive on this platform.
fn archive_ext() -> &'static str {
    if env::consts::OS == "windows" {
        "zip"
    } else {
        "tar.gz"
    }
}

/// GitHub release asset metadata.
#[derive(Debug)]
struct Asset {
    name: String,
    url: String,
}

/// GitHub release metadata.
#[derive(Debug)]
struct Release {
    tag: String,
    assets: Vec<Asset>,
}

/// Query the GitHub API for the latest release.
fn fetch_latest_release() -> Result<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .user_agent("synapse-updater")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp = client.get(&url).send()?;
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API returned {}", resp.status()));
    }

    let body: serde_json::Value = resp.json()?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("missing tag_name in release"))?
        .to_string();

    let assets = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("missing assets in release"))?
        .iter()
        .map(|a| Asset {
            name: a["name"].as_str().unwrap_or("").to_string(),
            url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
        })
        .filter(|a| !a.name.is_empty() && !a.url.is_empty())
        .collect();

    Ok(Release { tag, assets })
}

/// Download an asset from its URL.
fn download_asset(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("synapse-updater")
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        return Err(anyhow!("download failed: {}", resp.status()));
    }
    let bytes = resp.bytes()?.to_vec();
    Ok(bytes)
}

/// Extract a .tar.gz or .zip archive.
fn extract_archive(archive: &PathBuf, dest: &PathBuf, name: &str) -> Result<()> {
    if name.ends_with(".tar.gz") {
        // Use tar command (available on Linux and macOS).
        let status = std::process::Command::new("tar")
            .arg("xzf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
            .context("failed to run tar")?;
        if !status.success() {
            return Err(anyhow!("tar extraction failed"));
        }
    } else if name.ends_with(".zip") {
        // Use unzip or PowerShell Expand-Archive.
        let status = std::process::Command::new("unzip")
            .arg("-o")
            .arg(archive)
            .arg("-d")
            .arg(dest)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // Fallback: PowerShell.
                let ps_status = std::process::Command::new("powershell")
                    .arg("-NoProfile")
                    .arg("-Command")
                    .arg(format!(
                        "Expand-Archive -Force '{}' '{}'",
                        archive.display(),
                        dest.display()
                    ))
                    .status()
                    .context("failed to run PowerShell Expand-Archive")?;
                if !ps_status.success() {
                    return Err(anyhow!("zip extraction failed"));
                }
            }
        }
    } else {
        return Err(anyhow!("unknown archive format: {name}"));
    }
    Ok(())
}

/// Find the synapse binary inside the extracted directory.
fn find_binary(dir: &PathBuf) -> Result<PathBuf> {
    let exe_name = if env::consts::OS == "windows" {
        "synapse.exe"
    } else {
        "synapse"
    };

    // Search recursively for the binary.
    fn search(dir: &PathBuf, exe: &str) -> Option<PathBuf> {
        if dir.join(exe).exists() {
            return Some(dir.join(exe));
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(found) = search(&path, exe) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    search(dir, exe_name).ok_or_else(|| anyhow!("binary '{exe_name}' not found in archive"))
}

/// Install the binary by replacing the current executable.
fn install_binary(src: &PathBuf) -> Result<()> {
    let current_exe = env::current_exe().context("cannot determine current executable")?;

    // On Windows, we can't overwrite a running executable directly.
    if env::consts::OS == "windows" {
        let backup = current_exe.with_extension("exe.bak");
        if current_exe.exists() {
            let _ = fs::rename(&current_exe, &backup);
        }
        fs::copy(src, &current_exe).context("failed to install new binary")?;
        let _ = fs::remove_file(&backup);
    } else {
        // On Unix, write to a temp file then atomically rename.
        let tmp = current_exe.with_extension("new");
        fs::copy(src, &tmp).context("failed to copy new binary")?;

        // Preserve permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&tmp, perms)?;
        }

        fs::rename(&tmp, &current_exe).context("failed to replace binary")?;
    }

    print!("  installing…");
    io::stdout().flush()?;
    Ok(())
}
