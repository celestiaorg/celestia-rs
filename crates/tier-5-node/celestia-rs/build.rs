//! Embeds the git commit, build profile and toolchain for `celestia-rs
//! version --long` and `GetNodeInfo`.
//!
//! The commit comes from `git rev-parse HEAD` (suffixed `-dirty` when the
//! work tree has uncommitted changes). Builds outside a git checkout (source
//! tarballs) can set `CELESTIA_RS_GIT_COMMIT`; otherwise the commit is
//! `unknown`.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CELESTIA_RS_GIT_COMMIT");

    let commit = std::env::var("CELESTIA_RS_GIT_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            let head = git(&["rev-parse", "HEAD"])?;
            // Rebuild when HEAD moves (checkout, commit) or the index changes.
            if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
                let dir = PathBuf::from(dir);
                for f in ["HEAD", "index"] {
                    println!("cargo:rerun-if-changed={}", dir.join(f).display());
                }
                if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
                    println!("cargo:rerun-if-changed={}", dir.join(r).display());
                }
            }
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            Some(if dirty { format!("{head}-dirty") } else { head })
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=CELESTIA_RS_GIT_COMMIT={commit}");

    // `PROFILE` is only `debug` or `release`; the custom profile (e.g.
    // `dist`) is the name of the target directory component.
    println!("cargo:rerun-if-env-changed=CELESTIA_RS_BUILD_PROFILE");
    let inferred_profile = std::env::var("OUT_DIR")
        .ok()
        .and_then(|o| {
            let p = PathBuf::from(o);
            // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
            Some(p.ancestors().nth(3)?.file_name()?.to_str()?.to_owned())
        })
        .unwrap_or_else(|| std::env::var("PROFILE").unwrap_or_default());
    let profile = std::env::var("CELESTIA_RS_BUILD_PROFILE").unwrap_or(inferred_profile);
    println!("cargo:rustc-env=CELESTIA_RS_BUILD_PROFILE={profile}");

    // The version `celestia-rs version` reports: the crate's, or for a
    // daily release (.github/workflows/release.yml) the release's: a plain
    // `<major>.<minor>.<patch>` no lower than the crate version's (crate
    // 0.1.0 or 0.1.0-rc.9: 0.1.4 or 0.2.0, not 0.0.9).
    println!("cargo:rerun-if-env-changed=CELESTIA_RS_RELEASE_VERSION");
    let crate_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = match std::env::var("CELESTIA_RS_RELEASE_VERSION") {
        Ok(v) if !v.is_empty() => {
            let parse = |text: &str| -> Option<(u64, u64, u64)> {
                let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
                let parsed = (parts.next()??, parts.next()??, parts.next()??);
                parts.next().is_none().then_some(parsed)
            };
            let core = crate_version.split('-').next().unwrap_or_default();
            let valid = matches!(
                (parse(&v), parse(core)),
                (Some(release), Some(floor)) if release >= floor
            );
            assert!(
                valid,
                "CELESTIA_RS_RELEASE_VERSION={v} must be <major>.<minor>.<patch>, no lower than the crate version {crate_version}"
            );
            v
        }
        _ => crate_version,
    };
    println!("cargo:rustc-env=CELESTIA_RS_VERSION={version}");

    // The toolchain, like Go's `go version` line (`GetNodeInfo`).
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let toolchain = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "rustc".to_owned(), |v| v.trim().to_owned());
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=CELESTIA_RS_TOOLCHAIN={toolchain} {target}");
}
