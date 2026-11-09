use crate::{client, err, Result};
use semver::Version;
use serde::Deserialize;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset { name: String }

fn installer(version: &Version) -> String {
    let arm = cfg!(target_arch = "aarch64");
    let suffix = if cfg!(target_os = "macos") {
        if arm { "aarch64.dmg" } else { "x64.dmg" }
    } else if arm { "arm64-setup.exe" } else { "x64-setup.exe" };
    format!("Knotless_{version}_{suffix}")
}

fn newer_release(release: Release) -> Option<Version> {
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    let installed = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    if release.draft || release.prerelease || !version.pre.is_empty() || !version.build.is_empty() ||
        version <= installed { return None; }
    let name = installer(&version);
    release.assets.iter().any(|asset| asset.name == name).then_some(version)
}

#[tauri::command]
pub async fn check_app_update() -> Result<Option<String>> {
    let release = client()?.get("https://api.github.com/repos/rusynergy/Knotless/releases/latest")
        .timeout(std::time::Duration::from_secs(10)).send().await.map_err(err)?
        .error_for_status().map_err(err)?.json::<Release>().await.map_err(err)?;
    Ok(newer_release(release).map(|v| v.to_string()))
}

#[tauri::command]
pub fn open_app_update(version: String) -> Result<()> {
    let version = Version::parse(&version).map_err(err)?;
    let url = format!("https://github.com/rusynergy/Knotless/releases/download/v{version}/{}", installer(&version));
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let null = std::ptr::null();
        let result = unsafe { ShellExecuteW(std::ptr::null_mut(), null, wide.as_ptr(), null, null, 1) };
        if result as isize <= 32 { return Err(format!("ShellExecute: {}", result as isize)); }
    }
    #[cfg(target_os = "macos")]
    if !std::process::Command::new("/usr/bin/open").arg(url).status().map_err(err)?.success() {
        return Err("Could not open the browser".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_newer_stable_release_with_matching_installer() {
        let installed = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let next = Version::new(installed.major, installed.minor, installed.patch + 10);
        let release = |v: &Version, asset: String, prerelease| Release {
            tag_name: format!("v{v}"), draft: false, prerelease, assets: vec![Asset { name: asset }],
        };
        assert_eq!(newer_release(release(&next, installer(&next), false)), Some(next.clone()));
        assert_eq!(newer_release(release(&installed, installer(&installed), false)), None);
        assert_eq!(newer_release(release(&next, "SHA256SUMS.txt".into(), false)), None);
        assert_eq!(newer_release(release(&next, installer(&next), true)), None);
    }
}
