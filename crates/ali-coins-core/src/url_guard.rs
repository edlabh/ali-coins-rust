//! Guarda anti-SSRF compatível com `libs/url_guard.js` do oráculo.
//!
//! - Bloqueio de loopback/privados/link-local/metadata em IPv4 e IPv6
//!   (incluindo NAT64, 6to4, Teredo e IPv4 mapeado/compatível).
//! - Lista de hostnames privados (`localhost`, `ip6-localhost`, ...).
//! - Resolução DNS fail-closed validando **todos** os registros.
//! - `ALLOW_PRIVATE_WEBHOOKS` como opt-in explícito.
//!
//! `safe_fetch` (redirects revalidados + pinning por conexão) entra junto com o
//! cliente HTTP de notificações; esta entrega cobre a classificação e validação.

use crate::config::EnvSource;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs as _};
use url::{Host, Url};

const PRIVATE_HOSTNAMES: [&str; 4] = [
    "localhost",
    "localhost.localdomain",
    "ip6-localhost",
    "ip6-loopback",
];

/// Resultado da validação de URL externa.
#[derive(Debug, Clone)]
pub struct UrlValidation {
    /// Destino permitido?
    pub ok: bool,
    /// Motivo do bloqueio (mensagem PT-BR do oráculo).
    pub reason: Option<String>,
    /// URL parseada, quando aplicável.
    pub url: Option<Url>,
}

impl UrlValidation {
    fn allowed(url: Url) -> Self {
        Self {
            ok: true,
            reason: None,
            url: Some(url),
        }
    }

    fn blocked(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            url: None,
        }
    }
}

/// IP privado/loopback/link-local/reservado? (entrada não-IP → insegura)
#[must_use]
pub fn is_private_ip(raw: &str) -> bool {
    let Ok(ip) = raw.parse::<IpAddr>() else {
        return true;
    };
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    matches!(a, 10 | 127 | 0)
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    // IPv4 mapeado (::ffff:a.b.c.d).
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_private_v4(mapped);
    }
    let segments = ip.segments();
    let first = segments[0];
    if (first & 0xffc0) == 0xfe80 {
        return true; // link-local fe80::/10
    }
    if (first & 0xffc0) == 0xfec0 {
        return true; // site-local (depreciado) fec0::/10
    }
    if (first & 0xfe00) == 0xfc00 {
        return true; // unique local fc00::/7
    }
    if (first & 0xff00) == 0xff00 {
        return true; // multicast ff00::/8
    }
    if first == 0x0064 && segments[1] == 0xff9b {
        // NAT64 (64:ff9b::/96 e 64:ff9b:1::/48): valida o IPv4 embutido.
        return is_private_v4(trailing_ipv4(ip));
    }
    if first == 0x2002 {
        // 6to4: IPv4 nos hextets 2-3.
        let hi = segments[1];
        let lo = segments[2];
        return is_private_v4(Ipv4Addr::new(
            (hi >> 8) as u8,
            (hi & 0xff) as u8,
            (lo >> 8) as u8,
            (lo & 0xff) as u8,
        ));
    }
    if first == 0x2001 && segments[1] == 0 {
        // Teredo 2001::/32: valida o IPv4 dos 32 bits finais.
        return is_private_v4(trailing_ipv4(ip));
    }
    // IPv4 mapeado (::ffff:a.b.c.d) ou compatível (::a.b.c.d).
    if segments[..6].iter().all(|segment| *segment == 0) {
        let hi = segments[6];
        let lo = segments[7];
        return is_private_v4(Ipv4Addr::new(
            (hi >> 8) as u8,
            (hi & 0xff) as u8,
            (lo >> 8) as u8,
            (lo & 0xff) as u8,
        ));
    }
    false
}

fn trailing_ipv4(ip: Ipv6Addr) -> Ipv4Addr {
    let segments = ip.segments();
    let hi = segments[6];
    let lo = segments[7];
    Ipv4Addr::new(
        (hi >> 8) as u8,
        (hi & 0xff) as u8,
        (lo >> 8) as u8,
        (lo & 0xff) as u8,
    )
}

/// Mantém apenas endereços que NÃO são privados/loopback/metadata.
#[must_use]
pub fn filter_safe_addresses(addresses: &[String]) -> Vec<String> {
    addresses
        .iter()
        .filter(|address| !is_private_ip(address))
        .cloned()
        .collect()
}

/// `ALLOW_PRIVATE_WEBHOOKS` (true/1/on/yes, case-insensitive).
#[must_use]
pub fn allow_private_targets(env: &EnvSource) -> bool {
    crate::config::allow_private_targets(env)
}

/// Valida uma URL de destino externo quanto a SSRF.
#[must_use]
pub fn validate_external_url(
    raw_url: Option<&str>,
    allow_private: Option<bool>,
    resolve_dns: bool,
    env: &EnvSource,
) -> UrlValidation {
    let Some(raw_url) = raw_url.filter(|value| !value.is_empty()) else {
        return UrlValidation::blocked("URL ausente ou inválida.");
    };
    let Ok(parsed) = Url::parse(raw_url) else {
        return UrlValidation::blocked("URL malformada.");
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return UrlValidation::blocked(format!("Protocolo não permitido: {}:", parsed.scheme()));
    }

    let allow_private = allow_private.unwrap_or_else(|| allow_private_targets(env));
    let host = match parsed.host() {
        Some(Host::Domain(domain)) => domain
            .to_ascii_lowercase()
            .trim_end_matches('.')
            .to_string(),
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => ip.to_string(),
        None => return UrlValidation::blocked("URL malformada."),
    };

    if PRIVATE_HOSTNAMES.contains(&host.as_str()) {
        if !allow_private {
            return UrlValidation::blocked("Destino loopback bloqueado (SSRF).");
        }
        return UrlValidation::allowed(parsed);
    }

    if host.parse::<IpAddr>().is_ok() {
        if is_private_ip(&host) && !allow_private {
            return UrlValidation::blocked("Destino em rede privada/loopback bloqueado (SSRF).");
        }
        return UrlValidation::allowed(parsed);
    }

    if resolve_dns && !allow_private {
        let resolved: Vec<IpAddr> = match (host.as_str(), 0_u16).to_socket_addrs() {
            Ok(addresses) => addresses.map(|address| address.ip()).collect(),
            Err(err) => {
                return UrlValidation::blocked(format!(
                    "Não foi possível resolver o DNS do destino ({err}) — bloqueado por segurança (SSRF)."
                ));
            }
        };
        if resolved.is_empty() {
            return UrlValidation::blocked(
                "Destino sem registros DNS — bloqueado por segurança (SSRF).",
            );
        }
        for address in resolved {
            if is_private_ip(&address.to_string()) {
                return UrlValidation::blocked(format!(
                    "Host resolve para IP privado ({address}) — bloqueado (SSRF)."
                ));
            }
        }
    }

    UrlValidation::allowed(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_privados_e_publicos() {
        for ip in [
            "10.0.0.1",
            "127.0.0.1",
            "0.0.0.0",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.64.0.1",
            "192.0.0.1",
            "192.0.2.5",
            "192.88.99.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(is_private_ip(ip), "{ip} deveria ser privado");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",
            "192.169.0.1",
            "100.128.0.1",
        ] {
            assert!(!is_private_ip(ip), "{ip} deveria ser público");
        }
    }

    #[test]
    fn ipv6_privados_e_publicos() {
        for ip in [
            "::1",
            "::",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:7f00:1",
            "::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::",
        ] {
            assert!(is_private_ip(ip), "{ip} deveria ser privado");
        }
        for ip in [
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "2002:0808:0808::",
            // Teredo com IPv4 público embutido não é privado.
            "2001:0000:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(!is_private_ip(ip), "{ip} deveria ser público");
        }
        assert!(is_private_ip("nao-e-ip"));
    }

    #[test]
    fn valida_urls_sem_dns() {
        let env = EnvSource::default();
        let blocked = |raw: &str| {
            let result = validate_external_url(Some(raw), None, false, &env);
            assert!(!result.ok, "{raw} deveria ser bloqueada");
        };
        blocked("ftp://example.com/x");
        blocked("http://localhost/x");
        blocked("http://localhost./x");
        blocked("http://127.0.0.1/x");
        blocked("http://[::1]/x");
        blocked("http://169.254.169.254/latest/meta-data");
        blocked("nao-e-url");

        for allowed in ["https://example.com/hook", "http://8.8.8.8/x"] {
            let result = validate_external_url(Some(allowed), None, false, &env);
            assert!(result.ok, "{allowed} deveria ser permitida: {result:?}");
        }

        // Opt-in permite loopback.
        let result =
            validate_external_url(Some("http://127.0.0.1:8080/x"), Some(true), false, &env);
        assert!(result.ok);
    }
}
