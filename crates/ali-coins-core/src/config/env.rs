//! Fonte de variáveis de ambiente e coerções fiéis ao schema Zod do oráculo.
//!
//! As funções replicam as três famílias de booleanos, os inteiros
//! positivos/não-negativos e as particularidades de `Number()`/`parseInt`.

use std::collections::BTreeMap;

/// Mapa imutável de variáveis de ambiente (permite testes sem mexer em `std::env`).
#[derive(Debug, Clone, Default)]
pub struct EnvSource {
    vars: BTreeMap<String, String>,
}

impl EnvSource {
    /// Coleta o ambiente do processo atual.
    #[must_use]
    pub fn from_current_process() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Cria a partir de pares (testes/fixtures).
    #[must_use]
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Valor da variável, se presente.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }
}

/// `Number(valor)` do JavaScript (trim, hex, Infinity, vazio = 0).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn js_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "infinity" | "+infinity" => return Some(f64::INFINITY),
        "-infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    if let Some(hex) = lower.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }
    if let Some(hex) = lower.strip_prefix("-0x") {
        return u64::from_str_radix(hex, 16).ok().map(|v| -(v as f64));
    }
    trimmed.parse::<f64>().ok()
}

/// `positiveInt(default)`: inteiro > 0 ou default.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn positive_int(raw: Option<&str>, default: u64) -> u64 {
    match raw {
        None => default,
        Some(value) => match js_number(value) {
            Some(n) if n.is_finite() && n.fract() == 0.0 && n > 0.0 => {
                if n >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    n as u64
                }
            }
            _ => default,
        },
    }
}

/// `nonNegativeInt(default)`: inteiro >= 0 ou default.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn non_negative_int(raw: Option<&str>, default: u64) -> u64 {
    match raw {
        None => default,
        Some(value) => match js_number(value) {
            Some(n) if n.is_finite() && n.fract() == 0.0 && n >= 0.0 => {
                if n >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    n as u64
                }
            }
            _ => default,
        },
    }
}

/// Família A (`true`/`1` exatos, sem trim; resto = false) usada por
/// `ALLOW_MEDIA`, `NO_SANDBOX`, `TELEGRAM_*`, `TASK_RETRY_UNFINISHED`, `HEARTBEAT_ENABLED`.
#[must_use]
pub fn bool_true_exact(raw: Option<&str>) -> bool {
    match raw {
        None => false,
        Some(value) => value.eq_ignore_ascii_case("true") || value == "1",
    }
}

/// `ENCRYPT_LOCAL_SESSION`: default true; `false/0/off/no` (case-insensitive, com trim) desligam.
#[must_use]
pub fn bool_encrypt_local(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(value) => {
            let trimmed = value.trim().to_ascii_lowercase();
            !matches!(trimmed.as_str(), "false" | "0" | "off" | "no")
        }
    }
}

/// `HEADLESS`: default true; apenas `false`/`0` desligam.
#[must_use]
pub fn bool_headless(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(value) => !value.eq_ignore_ascii_case("false") && value != "0",
    }
}

/// `SKIP_APP_ONLY_TASKS`: default true; listas explícitas ou `Boolean(val)`.
#[must_use]
pub fn bool_skip_app_only(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(value) => {
            let trimmed = value.trim().to_ascii_lowercase();
            if ["false", "0", "off", "no"].contains(&trimmed.as_str()) {
                false
            } else if ["true", "1", "on", "yes"].contains(&trimmed.as_str()) {
                true
            } else {
                !value.is_empty()
            }
        }
    }
}

/// `CAPTCHA_COOLDOWN_HOURS`: inteiro >= 0; vazio/inválido = 12.
///
/// Mantida para uso futuro/quando o upstream corrigir D-01
/// (`docs/05-divergencias-conhecidas.md`); hoje o `Config` replica o default 12.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn captcha_cooldown_hours(raw: Option<&str>) -> u64 {
    match raw {
        None => 12,
        Some(value) if value.trim().is_empty() => 12,
        Some(value) => match js_number(value) {
            Some(n) if n.is_finite() && n.fract() == 0.0 && n >= 0.0 => {
                if n >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    n as u64
                }
            }
            _ => 12,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inteiros_positivos_e_nao_negativos() {
        assert_eq!(positive_int(None, 25), 25);
        assert_eq!(positive_int(Some(""), 25), 25);
        assert_eq!(positive_int(Some("10"), 25), 10);
        assert_eq!(positive_int(Some("10abc"), 25), 25);
        assert_eq!(positive_int(Some("-3"), 25), 25);
        assert_eq!(positive_int(Some("0"), 25), 25);
        assert_eq!(positive_int(Some(" 42 "), 25), 42);
        assert_eq!(positive_int(Some("1e3"), 25), 1000);
        assert_eq!(positive_int(Some("0x10"), 25), 16);
        assert_eq!(non_negative_int(Some("0"), 10), 0);
        assert_eq!(non_negative_int(Some("-1"), 10), 10);
    }

    #[test]
    fn familias_de_booleanos() {
        assert!(bool_true_exact(Some("true")));
        assert!(bool_true_exact(Some("1")));
        assert!(bool_true_exact(Some("TRUE")));
        assert!(!bool_true_exact(Some(" true")));
        assert!(!bool_true_exact(Some("on")));
        assert!(!bool_true_exact(None));

        assert!(bool_encrypt_local(None));
        assert!(!bool_encrypt_local(Some("false")));
        assert!(!bool_encrypt_local(Some(" OFF ")));
        assert!(bool_encrypt_local(Some("qualquer")));

        assert!(bool_headless(None));
        assert!(!bool_headless(Some("false")));
        assert!(!bool_headless(Some("0")));
        assert!(bool_headless(Some("False ")));

        assert!(bool_skip_app_only(None));
        assert!(!bool_skip_app_only(Some("no")));
        assert!(bool_skip_app_only(Some("yes")));
        assert!(bool_skip_app_only(Some("talvez")));
        assert!(!bool_skip_app_only(Some("")));
    }

    #[test]
    fn cooldown_de_captcha() {
        assert_eq!(captcha_cooldown_hours(None), 12);
        assert_eq!(captcha_cooldown_hours(Some("")), 12);
        assert_eq!(captcha_cooldown_hours(Some("0")), 0);
        assert_eq!(captcha_cooldown_hours(Some("6")), 6);
        assert_eq!(captcha_cooldown_hours(Some("abc")), 12);
        assert_eq!(captcha_cooldown_hours(Some("-1")), 12);
    }
}
