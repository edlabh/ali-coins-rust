//! Parâmetros scrypt: sanitização defensiva e default efetivo (cgroup/RAM).

use crate::crypto::{
    SCRYPT_DEFAULT_N, SCRYPT_LOW_MEMORY_DEFAULT_N, SCRYPT_LOW_MEMORY_TOTAL_BYTES, SCRYPT_MAX_N,
    SCRYPT_MAX_P, SCRYPT_MAX_R, SCRYPT_MEMORY_CAP_BYTES, SCRYPT_MIN_N,
};

/// Parâmetros scrypt efetivos (N, r, p) e teto de memória do OpenSSL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScryptConfig {
    /// Custo (potência de 2).
    pub n: u32,
    /// Blocos por iteração.
    pub r: u32,
    /// Paralelismo.
    pub p: u32,
    /// Teto de memória (bytes) aceito pela derivação.
    pub maxmem: u64,
}

/// Parâmetros legados usados por v1/v2 (`N=16384, r=8, p=1, maxmem=64MB`).
pub const LEGACY_SCRYPT: ScryptConfig = ScryptConfig {
    n: 16_384,
    r: 8,
    p: 1,
    maxmem: 64 * 1024 * 1024,
};

/// Fallback dinâmico de v3: N efetivo da máquina, r=8, p=1, teto de 256 MB.
pub(crate) fn v3_fallback_config() -> ScryptConfig {
    ScryptConfig {
        n: effective_default_scrypt_n(),
        r: 8,
        p: 1,
        maxmem: SCRYPT_MEMORY_CAP_BYTES,
    }
}

/// Sanitiza N/r/p de entrada não confiável, reproduzindo `sanitizeScryptParams`.
///
/// `None` representa parseInt inválido/ausente; valores `<= 0` ou abaixo do
/// piso caem no fallback. Aplica os tetos e reduz N à metade até caber no cap
/// de 256 MB (fallback final `N=16384, r=1`).
#[must_use]
pub fn sanitize_scrypt_params(
    raw_n: Option<u64>,
    raw_r: Option<u64>,
    raw_p: Option<u64>,
    fallback: ScryptConfig,
) -> ScryptConfig {
    fn clamp_int(value: Option<u64>, min: u64, max: u64, default: u64) -> u64 {
        match value {
            None | Some(0) => default,
            Some(v) if v < min => default,
            Some(v) => v.min(max),
        }
    }
    fn memory_bytes(n: u64, r: u64, p: u64) -> u64 {
        128 * n * r * p + 128 * r * p
    }

    let mut n = u32::try_from(clamp_int(
        raw_n,
        u64::from(SCRYPT_MIN_N),
        u64::from(SCRYPT_MAX_N),
        u64::from(fallback.n),
    ))
    .unwrap_or(fallback.n);
    let mut r = u32::try_from(clamp_int(
        raw_r,
        1,
        u64::from(SCRYPT_MAX_R),
        u64::from(fallback.r),
    ))
    .unwrap_or(fallback.r);
    let p = u32::try_from(clamp_int(
        raw_p,
        1,
        u64::from(SCRYPT_MAX_P),
        u64::from(fallback.p),
    ))
    .unwrap_or(fallback.p);

    while n > SCRYPT_MIN_N
        && memory_bytes(u64::from(n), u64::from(r), u64::from(p)) > SCRYPT_MEMORY_CAP_BYTES
    {
        n >>= 1;
    }
    if memory_bytes(u64::from(n), u64::from(r), u64::from(p)) > SCRYPT_MEMORY_CAP_BYTES {
        n = SCRYPT_MIN_N;
        r = 1;
    }

    ScryptConfig {
        n,
        r,
        p,
        maxmem: SCRYPT_MEMORY_CAP_BYTES
            .min(memory_bytes(u64::from(n), u64::from(r), u64::from(p)) * 2),
    }
}

/// Default de N para **novas** cifragens (cgroup/RAM aware, com `SCRYPT_N`).
///
/// Não afeta a leitura de tokens existentes: em v3/v2 o custo vem do próprio
/// token (v2: legado; v3: embutido ou, no formato compacto, este default).
#[must_use]
pub fn effective_default_scrypt_n() -> u32 {
    effective_default_scrypt_n_with(physical_memory_bytes())
}

pub(crate) fn effective_default_scrypt_n_with(physical: Option<u64>) -> u32 {
    if let Some(env_n) = env_scrypt_n() {
        return env_n;
    }
    let limit = match physical {
        Some(total) => total.min(cgroup_memory_limit_bytes().unwrap_or(u64::MAX)),
        None => cgroup_memory_limit_bytes().unwrap_or(u64::MAX),
    };
    if limit <= SCRYPT_LOW_MEMORY_TOTAL_BYTES {
        SCRYPT_LOW_MEMORY_DEFAULT_N
    } else {
        SCRYPT_DEFAULT_N
    }
}

/// `SCRYPT_N` com a mesma semântica do getter do oráculo (piso vira default).
fn env_scrypt_n() -> Option<u32> {
    let raw = std::env::var("SCRYPT_N").ok()?;
    let parsed = js_parse_int(&raw)?;
    if parsed <= 0 {
        return None;
    }
    let parsed = u64::try_from(parsed).ok()?;
    if parsed < u64::from(SCRYPT_MIN_N) {
        return Some(SCRYPT_DEFAULT_N);
    }
    Some(u32::try_from(parsed.min(u64::from(u32::MAX))).unwrap_or(u32::MAX))
}

/// `parseInt` do JavaScript (aceita sinal, ignora whitespace inicial e sufixo).
pub(crate) fn js_parse_int(input: &str) -> Option<i64> {
    let s = input.trim_start();
    let bytes = s.as_bytes();
    let mut index = 0_usize;
    let mut sign = 1_i128;
    match bytes.first() {
        Some(b'+') => index = 1,
        Some(b'-') => {
            sign = -1;
            index = 1;
        }
        _ => {}
    }
    let start = index;
    let mut value: i128 = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(bytes[index] - b'0'));
        index += 1;
    }
    if index == start {
        return None;
    }
    let signed = value.saturating_mul(sign);
    let clamped = signed.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    Some(i64::try_from(clamped).unwrap_or(i64::MAX))
}

fn physical_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
                return Some(kb * 1024);
            }
        }
    }
    None
}

/// Limite de memória do cgroup (v2 `memory.max` ou v1 `memory.limit_in_bytes`).
pub(crate) fn cgroup_memory_limit_bytes() -> Option<u64> {
    for file in [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ] {
        let Ok(raw) = std::fs::read_to_string(file) else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "max" {
            continue;
        }
        if let Ok(value) = trimmed.parse::<u64>() {
            if value > 0 && value < 1_000_000_000_000_000 {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piso_e_tetos_de_n() {
        let fallback = ScryptConfig {
            n: 131_072,
            r: 8,
            p: 1,
            maxmem: SCRYPT_MEMORY_CAP_BYTES,
        };
        // Abaixo do piso -> fallback.
        assert_eq!(
            sanitize_scrypt_params(Some(8_000), None, None, fallback).n,
            131_072
        );
        // Ausente -> fallback.
        assert_eq!(
            sanitize_scrypt_params(None, None, None, fallback).n,
            131_072
        );
        // 0 -> fallback (mesma semântica de valor <= 0).
        assert_eq!(
            sanitize_scrypt_params(Some(0), None, None, fallback).n,
            131_072
        );

        // Acima do teto: o clamp aplica 2^20 e o cap de memória reduz à metade
        // até caber (com r=8, p=1 => 2^17), exatamente como o oráculo.
        let capped = sanitize_scrypt_params(Some(9_999_999), None, None, fallback);
        assert!(capped.n <= SCRYPT_MAX_N);
        assert_eq!(capped.n, 131_072);
        assert!(
            128 * u64::from(capped.n) * u64::from(capped.r) * u64::from(capped.p)
                + 128 * u64::from(capped.r) * u64::from(capped.p)
                <= SCRYPT_MEMORY_CAP_BYTES
        );
    }

    #[test]
    fn teto_de_memoria_reduz_n_pela_metade() {
        let fallback = LEGACY_SCRYPT;
        // N=2^20, r=16, p=16 estoura o cap e deve ser reduzido até caber.
        let config = sanitize_scrypt_params(Some(1_048_576), Some(16), Some(16), fallback);
        let memory = 128 * u64::from(config.n) * u64::from(config.r) * u64::from(config.p)
            + 128 * u64::from(config.r) * u64::from(config.p);
        assert!(memory <= SCRYPT_MEMORY_CAP_BYTES, "memoria={memory}");
        assert!(config.n >= SCRYPT_MIN_N);
        // maxmem cobre a memória real com 2x de headroom, limitado ao cap.
        assert_eq!(config.maxmem, SCRYPT_MEMORY_CAP_BYTES.min(memory * 2));
    }

    #[test]
    fn r_e_p_invalidos_caem_no_fallback() {
        let fallback = LEGACY_SCRYPT;
        // r=0 cai no fallback (8); p=999 é clampado a 16. Com N no piso e
        // r*p alto, o cap de memória força o fallback final (N=16384, r=1).
        let config = sanitize_scrypt_params(Some(16_384), Some(0), Some(999), fallback);
        assert_eq!(config.p, SCRYPT_MAX_P);
        assert_eq!(config.n, SCRYPT_MIN_N);
        assert_eq!(config.r, 1);

        // Sem estouro de memória, o fallback de r é preservado.
        let config = sanitize_scrypt_params(Some(16_384), Some(0), Some(1), fallback);
        assert_eq!(config.r, fallback.r);
        assert_eq!(config.p, 1);
    }

    #[test]
    fn default_efetivo_respeita_ram_baixa() {
        assert_eq!(
            effective_default_scrypt_n_with(Some(512 * 1024 * 1024)),
            SCRYPT_LOW_MEMORY_DEFAULT_N
        );
        assert_eq!(
            effective_default_scrypt_n_with(Some(8_u64 * 1024 * 1024 * 1024)),
            SCRYPT_DEFAULT_N
        );
    }

    #[test]
    fn js_parse_int_comportamento() {
        assert_eq!(js_parse_int("16384"), Some(16_384));
        assert_eq!(js_parse_int("  16384abc"), Some(16_384));
        assert_eq!(js_parse_int("+42"), Some(42));
        assert_eq!(js_parse_int("-7"), Some(-7));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(js_parse_int(""), None);
    }
}
