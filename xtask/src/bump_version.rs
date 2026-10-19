//! `cargo xtask bump-version <VERSION>`: set the version the `celestia-rs`
//! binary reports (and releases are tagged with) to VERSION.
//!
//! Only the binary crate is versioned: every other workspace crate is
//! `publish = false` and stays at 0.0.1. The release workflows
//! (`.github/workflows/prepare-release.yml`, `release.yml`) run this, but it
//! works locally too. It edits, in place:
//!
//! - `crates/tier-5-node/celestia-rs/Cargo.toml`: `[package] version`;
//! - `Cargo.lock`: the `celestia-rs` package's version, so `--locked`
//!   builds keep working;
//! - `CHANGELOG.md`: `## Unreleased` becomes `## vVERSION (DATE)`, under a
//!   new, empty `## Unreleased`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

const MANIFEST: &str = "crates/tier-5-node/celestia-rs/Cargo.toml";
const PACKAGE: &str = "celestia-rs";

pub fn run(version: &str, date: Option<&str>, dry_run: bool) -> Result<()> {
    let version = version.strip_prefix('v').unwrap_or(version);
    check_version(version)?;
    let date = match date {
        Some(d) => d.to_string(),
        None => today(),
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask has no parent directory")?;

    let manifest = root.join(MANIFEST);
    let old = read(&manifest)?;
    let current = manifest_version(&old)?;
    if current == version {
        bail!("{PACKAGE} is already at {version}");
    }
    let edits = [
        (manifest, set_manifest_version(&old, version)?),
        (
            root.join("Cargo.lock"),
            set_lock_version(&read(&root.join("Cargo.lock"))?, version)?,
        ),
        (
            root.join("CHANGELOG.md"),
            cut_changelog(&read(&root.join("CHANGELOG.md"))?, version, &date)?,
        ),
    ];
    for (path, text) in &edits {
        if dry_run {
            println!("would update {}", path.display());
        } else {
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    println!("{PACKAGE} {current} -> {version}");
    Ok(())
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// MAJOR.MINOR.PATCH with an optional `-PRERELEASE` (`0.1.0`, `0.1.0-rc.2`);
/// no build metadata, which tags and the installer don't carry.
fn check_version(v: &str) -> Result<()> {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
    };
    let ok_pre = pre.is_none_or(|p| {
        p.split('.')
            .all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    });
    if parts.len() != 3 || !parts.iter().all(|p| numeric(p)) || !ok_pre {
        bail!("'{v}' is not a version like 0.1.0 or 0.1.0-rc.2");
    }
    Ok(())
}

/// The `version` of the manifest's `[package]` table.
fn manifest_version(manifest: &str) -> Result<String> {
    let (_, line) = package_version_line(manifest)?;
    Ok(line
        .split('"')
        .nth(1)
        .context("unquoted [package] version")?
        .to_string())
}

/// The index and text of the `version = "…"` line in `[package]`.
fn package_version_line(manifest: &str) -> Result<(usize, &str)> {
    let mut in_package = false;
    for (i, line) in manifest.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
        } else if in_package && t.starts_with("version") && t.contains('=') {
            return Ok((i, line));
        }
    }
    bail!("no version in the [package] table of {MANIFEST}")
}

fn set_manifest_version(manifest: &str, version: &str) -> Result<String> {
    let (index, _) = package_version_line(manifest)?;
    Ok(replace_line(
        manifest,
        index,
        &format!("version = \"{version}\""),
    ))
}

/// Sets the version of the `[[package]]` named `celestia-rs` in Cargo.lock.
fn set_lock_version(lock: &str, version: &str) -> Result<String> {
    let lines: Vec<&str> = lock.lines().collect();
    let name = format!("name = \"{PACKAGE}\"");
    for (i, line) in lines.iter().enumerate() {
        if *line == name && i > 0 && lines[i - 1] == "[[package]]" {
            if lines
                .get(i + 1)
                .is_some_and(|l| l.starts_with("version = "))
            {
                return Ok(replace_line(
                    lock,
                    i + 1,
                    &format!("version = \"{version}\""),
                ));
            }
            bail!("Cargo.lock: {PACKAGE} has no version line");
        }
    }
    bail!("Cargo.lock has no {PACKAGE} package")
}

fn replace_line(text: &str, index: usize, new: &str) -> String {
    let mut out: Vec<&str> = text.lines().collect();
    out[index] = new;
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Turns the first `## Unreleased` into the release's heading, with a new
/// empty `## Unreleased` above it.
fn cut_changelog(changelog: &str, version: &str, date: &str) -> Result<String> {
    let heading = format!("## v{version} ");
    if changelog.lines().any(|l| l.starts_with(&heading)) {
        bail!("CHANGELOG.md already has a v{version} section");
    }
    let Some(at) = changelog.find("\n## Unreleased\n") else {
        bail!("CHANGELOG.md has no '## Unreleased' heading");
    };
    let (head, rest) = changelog.split_at(at + 1);
    let rest = rest.strip_prefix("## Unreleased\n").unwrap_or(rest);
    Ok(format!(
        "{head}## Unreleased\n\n## v{version} ({date})\n{rest}"
    ))
}

/// Today's UTC date as YYYY-MM-DD.
fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(secs / 86_400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a Gregorian (year, month, day): Howard
/// Hinnant's `civil_from_days`, for dates from 1970 on.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_TEXT: &str = "[package]\nname = \"celestia-rs\"\nversion = \"0.0.1\"\nedition.workspace = true\n\n[dependencies]\nfoo = { version = \"1\" }\n";

    #[test]
    fn versions() {
        for ok in ["0.1.0", "0.1.0-rc.2", "1.10.0-alpha.1", "10.0.0"] {
            assert!(check_version(ok).is_ok(), "{ok}");
        }
        for bad in [
            "0.1",
            "v0.1.0",
            "0.1.0-",
            "01.0.0",
            "0.1.0+build",
            "0.1.0-rc..1",
            "",
        ] {
            assert!(check_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn manifest_only_package_version() {
        let out = set_manifest_version(MANIFEST_TEXT, "0.2.0").unwrap();
        assert!(out.contains("version = \"0.2.0\"\nedition"));
        assert!(out.contains("foo = { version = \"1\" }"));
        assert_eq!(manifest_version(&out).unwrap(), "0.2.0");
    }

    #[test]
    fn lock_only_this_package() {
        let lock = "[[package]]\nname = \"celestia-rs-x\"\nversion = \"0.0.1\"\n\n[[package]]\nname = \"celestia-rs\"\nversion = \"0.0.1\"\ndependencies = []\n";
        let out = set_lock_version(lock, "0.1.0-rc.2").unwrap();
        assert_eq!(
            out,
            "[[package]]\nname = \"celestia-rs-x\"\nversion = \"0.0.1\"\n\n[[package]]\nname = \"celestia-rs\"\nversion = \"0.1.0-rc.2\"\ndependencies = []\n"
        );
        assert!(set_lock_version("[[package]]\nname = \"other\"\n", "1.0.0").is_err());
    }

    #[test]
    fn changelog_cut() {
        let c = "# Changelog\n\n## Unreleased\n\n### Added\n\n- x\n";
        let out = cut_changelog(c, "0.1.0", "2026-10-19").unwrap();
        assert_eq!(
            out,
            "# Changelog\n\n## Unreleased\n\n## v0.1.0 (2026-10-19)\n\n### Added\n\n- x\n"
        );
        assert!(cut_changelog(&out, "0.1.0", "2026-10-20").is_err());
        assert!(cut_changelog("# Changelog\n", "0.1.0", "d").is_err());
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_745), (2026, 10, 19));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
