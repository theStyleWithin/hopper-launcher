//! Works out which release files are real builds for a given OS and CPU.
//! The rules are documented in DESIGN.md and mirrored in design/mockup.html.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Build {
    pub file: String,
    pub os: String,
    pub kind: String,
    pub arch: String,
    pub size: u64,
}

/// Kinds the launcher hands to the operating system instead of managing itself.
pub fn is_handoff(kind: &str) -> bool {
    matches!(kind, "installer" | "msi" | "deb" | "rpm" | "apk")
}

fn rank(kind: &str) -> u8 {
    match kind {
        "appimage" => 0,
        "deb" => 1,
        "rpm" => 2,
        "installer" => 0,
        "msi" => 1,
        "portable" => 2,
        "exe" => 3,
        "zip" => 4,
        "apk" => 0,
        _ => 9,
    }
}

/// Returns (os, kind, arch) for a release file, or None if it is not a build.
pub fn classify(file: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let f = file.to_lowercase();
    let skip = ["sha256", ".sig", ".asc", ".txt", ".tar.gz", "debug"];
    if skip.iter().any(|s| f.contains(s)) {
        return None;
    }
    let (os, kind) = if f.ends_with(".appimage") {
        ("linux", "appimage")
    } else if f.ends_with(".deb") {
        ("linux", "deb")
    } else if f.ends_with(".rpm") {
        ("linux", "rpm")
    } else if f.ends_with(".apk") {
        ("android", "apk")
    } else if f.ends_with(".msi") {
        ("windows", "msi")
    } else if f.ends_with(".exe") {
        if f.contains("setup") || f.contains("install") {
            ("windows", "installer")
        } else if f.contains("portable") {
            ("windows", "portable")
        } else {
            ("windows", "exe")
        }
    } else if f.ends_with(".zip") && f.contains("win") {
        ("windows", "zip")
    } else {
        return None;
    };
    let arch = if f.contains("aarch64") || f.contains("arm64") {
        "aarch64"
    } else if ["x86_64", "x64", "amd64", "win64"].iter().any(|s| f.contains(s)) {
        "x86_64"
    } else if os == "android" {
        "any"
    } else {
        "x86_64"
    };
    Some((os, kind, arch))
}

/// The builds in a release that suit this device, best first.
pub fn builds_for(assets: &[Asset], os: &str, arch: &str) -> Vec<Build> {
    let mut out: Vec<Build> = assets
        .iter()
        .filter_map(|a| {
            let (o, k, ar) = classify(&a.name)?;
            (o == os && (ar == "any" || ar == arch)).then(|| Build {
                file: a.name.clone(),
                os: o.into(),
                kind: k.into(),
                arch: ar.into(),
                size: a.size,
            })
        })
        .collect();
    out.sort_by(|a, b| rank(&a.kind).cmp(&rank(&b.kind)).then(a.file.cmp(&b.file)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset { name: n.to_string(), browser_download_url: String::new(), size: 1 })
            .collect()
    }

    /// File names from HopCast release v0.1.15.
    const HOPCAST: &[&str] = &[
        "HopCast-release-0.1.15-x86_64.AppImage",
        "HopCast-debug-0.1.15-x86_64.AppImage",
        "hopcast-source-0.1.15.tar.gz",
        "hopcast-bin-aur-0.1.15.tar.gz",
        "hopcast-flatpak-0.1.15.tar.gz",
        "SHA256SUMS",
        "HopCast-windows-x64-0.1.15.zip",
        "SHA256SUMS-windows",
        "HopCast-android-0.1.15.apk",
        "SHA256SUMS-android",
        "hopcast-fdroid-0.1.15.tar.gz",
    ];

    fn files(b: Vec<Build>) -> Vec<String> {
        b.into_iter().map(|b| b.file).collect()
    }

    #[test]
    fn hopcast_linux_skips_debug_and_packaging() {
        let b = builds_for(&assets(HOPCAST), "linux", "x86_64");
        assert_eq!(files(b), vec!["HopCast-release-0.1.15-x86_64.AppImage"]);
    }

    #[test]
    fn hopcast_windows_is_the_zip() {
        let b = builds_for(&assets(HOPCAST), "windows", "x86_64");
        assert_eq!(files(b.clone()), vec!["HopCast-windows-x64-0.1.15.zip"]);
        assert_eq!(b[0].kind, "zip");
    }

    #[test]
    fn hopcast_android_is_only_the_apk() {
        let b = builds_for(&assets(HOPCAST), "android", "aarch64");
        assert_eq!(files(b), vec!["HopCast-android-0.1.15.apk"]);
    }

    #[test]
    fn windows_orders_installer_portable_zip() {
        let a = assets(&[
            "App-0.1-win.zip",
            "App-0.1-portable.exe",
            "App-Setup-0.1.exe",
            "App.msi",
            "App-linux.AppImage",
        ]);
        let kinds: Vec<_> = builds_for(&a, "windows", "x86_64").into_iter().map(|b| b.kind).collect();
        assert_eq!(kinds, vec!["installer", "msi", "portable", "zip"]);
    }

    #[test]
    fn linux_never_sees_windows_files() {
        let a = assets(&["A-Setup.exe", "A-portable.exe", "A.AppImage", "A.apk"]);
        assert_eq!(files(builds_for(&a, "linux", "x86_64")), vec!["A.AppImage"]);
    }

    #[test]
    fn arm_devices_do_not_get_x86_files() {
        let a = assets(&["A-x86_64.AppImage", "A-aarch64.AppImage", "B.AppImage"]);
        assert_eq!(files(builds_for(&a, "linux", "aarch64")), vec!["A-aarch64.AppImage"]);
    }

    #[test]
    fn unknown_files_are_ignored() {
        assert!(classify("notes.md").is_none());
        assert!(classify("SHA256SUMS").is_none());
        assert!(classify("data.zip").is_none());
    }
}
