//! Small helpers shared by the CLI commands: console output, Go time
//! formatting, JSON pretty-printing with Go's indents, stdin lines.

use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};

/// Writes a line to stdout (CLI command output); a closed pipe (`| head`)
/// is not an error.
pub fn out(s: impl AsRef<str>) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{}", s.as_ref());
    let _ = o.flush();
}

/// Writes a line to stderr (prompts, notices).
pub fn err(s: impl AsRef<str>) {
    let _ = writeln!(std::io::stderr(), "{}", s.as_ref());
}

/// The default node home: `$CELESTIA_RS_HOME`, else `~/.celestia-rs`.
#[must_use]
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("CELESTIA_RS_HOME") {
        return PathBuf::from(h);
    }
    std::env::var_os("HOME")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(".celestia-rs")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Go `time.Time.Format(time.RFC3339Nano)` in UTC (trailing zeros of the
/// fraction trimmed).
#[must_use]
pub fn rfc3339_nano(secs: i64, nanos: u32) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    let mut s = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    );
    if nanos > 0 {
        let frac = format!("{nanos:09}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s.push('Z');
    s
}

/// The current time as [`rfc3339_nano`].
#[must_use]
pub fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    rfc3339_nano(
        i64::try_from(now.as_secs()).unwrap_or(0),
        now.subsec_nanos(),
    )
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// `json.MarshalIndent(v, "", indent)`-style pretty printing.
#[must_use]
pub fn json_indent(v: &serde_json::Value, indent: &str) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    let _ = serde::Serialize::serialize(v, &mut ser);
    String::from_utf8(buf).unwrap_or_default()
}

/// Go's `encoding/json` escapes `<`, `>` and `&` in strings.
#[must_use]
pub fn go_json_escape(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Reads one line from stdin (without the newline); `None` at EOF.
#[must_use]
pub fn read_stdin_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_owned()),
    }
}

/// Prompts on stderr and reads a line (Go `input.GetString` /
/// `input.GetPassword` read piped stdin the same way).
pub fn prompt(msg: &str) -> anyhow::Result<String> {
    let mut e = std::io::stderr();
    let _ = write!(e, "{msg}");
    let _ = e.flush();
    let line = read_stdin_line().ok_or_else(|| anyhow::anyhow!("EOF"))?;
    let _ = writeln!(e);
    Ok(line)
}

/// Writes `bytes` with mode 0600 (keys, genesis), creating parent dirs.
pub fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(rfc3339_nano(0, 0), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339_nano(1_792_000_080, 532_290_000),
            "2026-10-14T17:48:00.53229Z"
        );
    }
}
