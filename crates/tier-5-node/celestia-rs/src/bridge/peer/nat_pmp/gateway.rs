//! Discover the IPv4 default router of this network namespace.
use std::{io, net::Ipv4Addr};

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn default_gateway() -> io::Result<Ipv4Addr> {
    let routes = std::fs::read_to_string("/proc/net/route")?;
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (_, destination, gateway, flags) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            let metric = fields.nth(2)?.parse::<u32>().ok()?;
            let mask = fields.next()?;
            let flags = u32::from_str_radix(flags, 16).ok()?;
            if destination != "00000000" || mask != "00000000" || flags & 3 != 3 {
                return None;
            }
            let gateway = u32::from_str_radix(gateway, 16).ok()?;
            Some((metric, Ipv4Addr::from(gateway.to_ne_bytes())))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
        .ok_or_else(|| io::ErrorKind::NetworkUnreachable.into())
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub(super) fn default_gateway() -> io::Result<Ipv4Addr> {
    let output = std::process::Command::new("/sbin/route")
        .args(["-n", "get", "default"])
        .output()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("gateway:")?.trim().parse().ok())
        .ok_or_else(|| io::ErrorKind::NetworkUnreachable.into())
}

#[cfg(target_os = "windows")]
pub(super) fn default_gateway() -> io::Result<Ipv4Addr> {
    let root = std::env::var_os("SystemRoot").ok_or(io::ErrorKind::NotFound)?;
    let output = std::process::Command::new(std::path::Path::new(&root).join("System32/route.exe"))
        .args(["print", "-4"])
        .output()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != "0.0.0.0" || fields.next()? != "0.0.0.0" {
                return None;
            }
            let gateway = fields.next()?.parse().ok()?;
            let metric = fields.nth(1)?.parse::<u32>().ok()?;
            Some((metric, gateway))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
        .ok_or_else(|| io::ErrorKind::NetworkUnreachable.into())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "windows"
)))]
pub(super) fn default_gateway() -> io::Result<Ipv4Addr> {
    Err(io::ErrorKind::Unsupported.into())
}
