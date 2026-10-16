//! Embeds the git commit and build profile for `celestia-rs version --long`.
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
    let profile = std::env::var("OUT_DIR")
        .ok()
        .and_then(|o| {
            let p = PathBuf::from(o);
            // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
            Some(p.ancestors().nth(3)?.file_name()?.to_str()?.to_owned())
        })
        .unwrap_or_else(|| std::env::var("PROFILE").unwrap_or_default());
    println!("cargo:rustc-env=CELESTIA_RS_BUILD_PROFILE={profile}");
}
