//! Utilitários de tempo compatíveis com `time_utils.js` do oráculo.
//!
//! O "dia" do negócio usa `REPORT_TIMEZONE` (default `America/Los_Angeles`);
//! datas/horas de exibição, durações, backoff com jitter, pausas aleatórias e
//! espera abortável em pedaços seguem os mesmos formatos.

use crate::config::EnvSource;
use chrono::{DateTime, Datelike as _, Offset as _, Timelike as _, Utc};
use chrono_tz::Tz;
use std::str::FromStr as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Fuso padrão do relatório.
pub const DEFAULT_REPORT_TIMEZONE: &str = "America/Los_Angeles";

/// Fuso do relatório (lido de `REPORT_TIMEZONE`; inválido → UTC).
#[must_use]
pub fn report_timezone() -> Tz {
    static TIMEZONE: OnceLock<Tz> = OnceLock::new();
    *TIMEZONE.get_or_init(|| {
        let raw = std::env::var("REPORT_TIMEZONE")
            .unwrap_or_else(|_| DEFAULT_REPORT_TIMEZONE.to_string());
        Tz::from_str(raw.trim()).unwrap_or(chrono_tz::UTC)
    })
}

/// Data no formato `DD/MM/AAAA` no fuso do relatório.
#[must_use]
pub fn format_date(dt: DateTime<Utc>) -> String {
    let local = dt.with_timezone(&report_timezone());
    format!("{:02}/{:02}/{}", local.day(), local.month(), local.year())
}

/// Hora no formato `HH:mm:ss` no fuso do relatório.
#[must_use]
pub fn format_time(dt: DateTime<Utc>) -> String {
    let local = dt.with_timezone(&report_timezone());
    format!(
        "{:02}:{:02}:{:02}",
        local.hour(),
        local.minute(),
        local.second()
    )
}

/// Data e hora no formato `DD/MM/AAAA HH:mm:ss`.
#[must_use]
pub fn format_date_time(dt: DateTime<Utc>) -> String {
    format!("{} {}", format_date(dt), format_time(dt))
}

/// Duração legível: `45s`, `1m 20s`, `1h 01m 05s` (negativo/inválido → `0s`).
#[must_use]
pub fn format_duration(ms: i64) -> String {
    let ms = ms.max(0);
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Backoff exponencial com jitter (80%–120%), teto e base configurável.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn calculate_account_backoff(
    attempt: i64,
    base_ms: Option<u64>,
    max_ms: u64,
    jitter_fraction: f64,
    env: &EnvSource,
) -> u64 {
    let effective_base = base_ms.filter(|base| *base > 0).unwrap_or_else(|| {
        env.get("ACCOUNT_BACKOFF_BASE_MS")
            .and_then(crate::config::env::js_number)
            .filter(|value| *value > 0.0)
            .map_or(2000, |value| value as u64)
    });
    let safe_attempt = attempt.max(0).min(i64::from(i32::MAX)) as i32;
    let exponential = (effective_base as f64) * 2_f64.powi(safe_attempt);
    let capped = exponential.min(max_ms as f64);
    let jitter = 0.8 + 0.4 * jitter_fraction.clamp(0.0, 1.0);
    (capped * jitter).round().min(max_ms as f64) as u64
}

/// Combina backoff e pausa devolvendo a MAIOR espera (não soma).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn compose_account_wait_ms(backoff_ms: f64, delay_ms: f64) -> u64 {
    fn safe(value: f64) -> u64 {
        if value.is_finite() && value > 0.0 {
            value.floor() as u64
        } else {
            0
        }
    }
    safe(backoff_ms).max(safe(delay_ms))
}

/// Sorteia pausa uniforme e inclusiva entre `min` e `max` (0 desliga).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn pick_pause_ms(min_ms: f64, max_ms: f64, random_fraction: f64) -> u64 {
    let min = min_ms.max(0.0).floor();
    let max = max_ms.floor().max(min);
    if max == 0.0 {
        return 0;
    }
    (min + (random_fraction.clamp(0.0, 1.0) * (max - min + 1.0)).floor()) as u64
}

/// Espera até o instante-alvo, em pedaços de no máximo `chunk_ms`.
///
/// Devolve `false` se abortado via `abort`; `true` ao alcançar o alvo.
#[must_use]
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn wait_until_wall_clock(target_ms: i64, chunk_ms: u64, abort: Option<&AtomicBool>) -> bool {
    let chunk = chunk_ms.max(1) as i64;
    loop {
        if abort.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return false;
        }
        let remaining = target_ms - Utc::now().timestamp_millis();
        if remaining <= 0 {
            return true;
        }
        let sleep_ms = remaining.min(chunk).max(1) as u64;
        std::thread::sleep(Duration::from_millis(sleep_ms));
    }
}

/// Rótulo curto do fuso (aproximação de `Intl` short: `PDT`, `GMT-3`, `GMT+5:30`).
#[must_use]
pub fn get_report_timezone_label(dt: DateTime<Utc>) -> String {
    let tz = report_timezone();
    let local = dt.with_timezone(&tz);
    let offset_seconds = local.offset().fix().local_minus_utc();
    if tz == chrono_tz::America::Los_Angeles {
        return if offset_seconds == -7 * 3600 {
            "PDT".to_string()
        } else {
            "PST".to_string()
        };
    }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let absolute = offset_seconds.abs();
    let hours = absolute / 3600;
    let minutes = (absolute % 3600) / 60;
    if minutes == 0 {
        format!("GMT{sign}{hours}")
    } else {
        format!("GMT{sign}{hours}:{minutes:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    #[allow(clippy::many_single_char_names)]
    fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .unwrap()
    }

    #[test]
    fn formatacao_de_data_e_hora_em_la() {
        // 2026-01-15 12:34:56Z = 04:34:56 PST no mesmo dia.
        let winter = utc(2026, 1, 15, 12, 34, 56);
        assert_eq!(format_date(winter), "15/01/2026");
        assert_eq!(format_time(winter), "04:34:56");
        assert_eq!(format_date_time(winter), "15/01/2026 04:34:56");

        // 2026-06-15 07:00:00Z = 00:00:00 PDT; virada de dia em LA.
        let summer = utc(2026, 6, 15, 7, 0, 0);
        assert_eq!(format_date(summer), "15/06/2026");
        assert_eq!(format_time(summer), "00:00:00");
    }

    #[test]
    fn duracao() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45_000), "45s");
        assert_eq!(format_duration(80_000), "1m 20s");
        assert_eq!(format_duration(3_665_000), "1h 01m 05s");
        assert_eq!(format_duration(-5), "0s");
    }

    #[test]
    fn backoff_com_jitter() {
        let env = EnvSource::default();
        // attempt=0, base 2000, jitter 0.5 => fator 1.0 => 2000.
        assert_eq!(calculate_account_backoff(0, None, 30_000, 0.5, &env), 2000);
        // attempt=1 => 4000.
        assert_eq!(calculate_account_backoff(1, None, 30_000, 0.5, &env), 4000);
        // teto respeitado.
        assert_eq!(
            calculate_account_backoff(10, None, 30_000, 0.5, &env),
            30_000
        );
        // jitter mínimo (0) => 80%.
        assert_eq!(calculate_account_backoff(0, None, 30_000, 0.0, &env), 1600);
        // base por env.
        let env = EnvSource::from_pairs([("ACCOUNT_BACKOFF_BASE_MS", "1000")]);
        assert_eq!(calculate_account_backoff(0, None, 30_000, 0.5, &env), 1000);
    }

    #[test]
    fn pausa_aleatoria() {
        assert_eq!(pick_pause_ms(0.0, 0.0, 0.5), 0);
        assert_eq!(pick_pause_ms(1000.0, 2000.0, 0.0), 1000);
        assert_eq!(pick_pause_ms(1000.0, 2000.0, 0.999), 1999);
        // max < min usa min.
        assert_eq!(pick_pause_ms(5000.0, 1000.0, 0.0), 5000);
    }

    #[test]
    fn compose_usa_o_maior() {
        assert_eq!(compose_account_wait_ms(5000.0, 1000.0), 5000);
        assert_eq!(compose_account_wait_ms(0.0, 3000.0), 3000);
        assert_eq!(compose_account_wait_ms(f64::NAN, -5.0), 0);
    }

    #[test]
    fn espera_abortavel() {
        let abort = AtomicBool::new(false);
        let start = Utc::now().timestamp_millis();
        assert!(wait_until_wall_clock(start + 120, 50, Some(&abort)));
        assert!(Utc::now().timestamp_millis() >= start + 100);

        abort.store(true, Ordering::SeqCst);
        let start = Utc::now().timestamp_millis();
        assert!(!wait_until_wall_clock(start + 60_000, 1000, Some(&abort)));
    }

    #[test]
    fn rotulo_de_fuso() {
        assert_eq!(get_report_timezone_label(utc(2026, 1, 15, 12, 0, 0)), "PST");
        assert_eq!(get_report_timezone_label(utc(2026, 6, 15, 12, 0, 0)), "PDT");
    }
}
