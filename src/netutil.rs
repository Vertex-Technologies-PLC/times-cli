use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use crate::error::CliError;

pub fn is_public_https_url(url: &str) -> bool {
    validate_https_target(url, false).is_ok()
}

pub fn validate_probe_url(url: &str) -> anyhow::Result<reqwest::Url> {
    validate_https_target(url, true)
}

fn validate_https_target(url: &str, resolve_dns: bool) -> anyhow::Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        CliError::validation(format!("origin URL is not valid: {url}"))
    })?;
    if parsed.scheme() != "https" {
        return Err(CliError::validation("origin URL must be https://").into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CliError::validation("origin URL must not include credentials").into());
    }
    if let Some(port) = parsed.port() {
        if port != 443 {
            return Err(
                CliError::validation("origin URL must use the default HTTPS port 443").into(),
            );
        }
    }
    let Some(host) = parsed.host_str() else {
        return Err(CliError::validation("origin URL has no host").into());
    };
    let host_ip = host.trim().trim_matches('[').trim_end_matches(']');
    if looks_like_local_host(host) {
        return Err(CliError::validation(
            "origin URL must be a public hostname (no localhost or private IPs)",
        )
        .into());
    }
    if let Ok(ip) = host_ip.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(CliError::validation(
                "origin URL must not target a private, loopback, or link-local IP",
            )
            .into());
        }
        return Ok(parsed);
    }
    if resolve_dns {
        resolve_public_host(host)?;
    }
    Ok(parsed)
}

pub fn resolve_public_host(host: &str) -> anyhow::Result<()> {
    let addrs = (host, 443u16)
        .to_socket_addrs()
        .map_err(|e| CliError::network(format!("could not resolve {host}: {e}")))?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if !is_public_ip(addr.ip()) {
            return Err(CliError::validation(format!(
                "{host} resolves to a private or link-local address and cannot be used as an origin"
            ))
            .into());
        }
    }
    if !any {
        return Err(CliError::network(format!("{host} did not resolve")).into());
    }
    Ok(())
}

pub fn looks_like_local_host(host: &str) -> bool {
    let host = host.trim().trim_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host == "0.0.0.0"
        || host == "::"
        || host == "::1"
        || host == "127.0.0.1"
        || host == "metadata.google.internal"
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return false;
    }
    if ip.is_link_local() || ip.is_private() {
        return false;
    }
    let o = ip.octets();
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return false;
    }
    if o[0] == 192 && o[1] == 0 && o[2] == 2 {
        return false;
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let o = ip.octets();
    if ip.is_unicast_link_local() {
        return false;
    }
    if o[0] & 0xfe == 0xfc {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_https_accepts_example() {
        assert!(is_public_https_url("https://a.example.com/app"));
    }

    #[test]
    fn public_https_rejects_private_and_local() {
        assert!(!is_public_https_url("http://a.example.com"));
        assert!(!is_public_https_url("https://localhost/"));
        assert!(!is_public_https_url("https://dev.localhost/"));
        assert!(!is_public_https_url("https://127.0.0.1/"));
        assert!(!is_public_https_url("https://10.0.0.4/"));
        assert!(!is_public_https_url("https://192.168.1.9/"));
        assert!(!is_public_https_url("https://169.254.169.254/"));
        assert!(!is_public_https_url("https://user:pass@a.example.com/"));
        assert!(!is_public_https_url("https://a.example.com:8443/"));
    }

    #[test]
    fn private_ipv6_and_mapped() {
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("fc00::1".parse().unwrap()));
        assert!(!is_public_ip("fe80::1".parse().unwrap()));
        assert!(!is_public_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn resolve_rejects_loopback_host() {
        let err = resolve_public_host("localhost").unwrap_err();
        assert!(err.to_string().to_ascii_lowercase().contains("private")
            || err.to_string().to_ascii_lowercase().contains("link-local")
            || err.to_string().to_ascii_lowercase().contains("localhost"));
    }
}
