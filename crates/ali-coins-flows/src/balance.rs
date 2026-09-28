//! Parsers de saldo/streak/extrato compatíveis com `libs/ui/balance.js`.
//!
//! Funções puras (sem browser): regexes multilíngues de streak, mapa
//! moedas⇄dias do ciclo oficial, detecção de prompt de login e extração do
//! extrato de hoje (bônus de check-in vs. missões).

use chrono::Datelike as _;
use regex::Regex;
use std::sync::OnceLock;

/// Padrões multilíngues (pt/en/es) de sequência de dias.
pub const STREAK_PATTERNS: [&str; 9] = [
    r"(?i)se[cq]u[eê]ncia(?:\s*de|:)?\s*([0-9]+)\s*d[ií]as?",
    r"(?i)([0-9]+)\s*d[ií]as?\s*(?:de\s*se[cq]u[eê]ncia|seguidos?|consecutivos?)",
    r"(?i)([0-9]+)\s*-?\s*(?:day|dia|dias|days|días)?\s*-?\s*streak",
    r"(?i)streak(?:\s*de|:)?\s*([0-9]+)",
    r"(?i)([0-9]+)\s*(?:days?|dias?|días?)\s*(?:in\s*a\s*row|consecutivos?)",
    r"(?i)check-?in(?:\s*(?:de|por|di[aá]rio:?))?\s*([0-9]+)\s*d[ií]as?",
    r"(?i)([0-9]+)\s*d[ií]as?\s*de\s*check-?in",
    r"(?i)completou\s*([0-9]+)\s*d[ií]as?",
    r"(?i)coletou\s*por\s*([0-9]+)\s*d[ií]as?",
];

/// Rótulo do check-in no extrato (bilíngue).
pub const CHECKIN_LABEL_PATTERN: &str =
    r"(?i)(?:App daily check-in|Check-in di[áa]rio no app|B[ôo]nus di[áa]rio|Daily bonus)";

/// Resultado da extração do extrato de hoje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerExtract {
    /// Moedas de check-in somadas (`None` quando não houve lançamento).
    pub bonus_coins: Option<i64>,
    /// Moedas das demais missões.
    pub missions_coins: i64,
    /// Quantidade de lançamentos de check-in.
    pub bonus_count: u32,
    /// Quantidade de lançamentos de missões.
    pub missions_count: u32,
}

/// O texto é um prompt de login/reautenticação (inclui o fluxo SPA só-senha).
#[must_use]
pub fn is_login_prompt_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    lower.contains("sign in with email code")
        || lower.contains("switch account")
        || lower.contains("trocar de conta")
        || ((lower.contains("forgot password") || lower.contains("esqueci minha senha"))
            && (lower.contains("password") || lower.contains("senha")))
}

/// Extrai o número de dias de sequência de um texto (primeiro padrão que casar).
#[must_use]
pub fn extract_streak_from_text(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    for pattern in STREAK_PATTERNS {
        let regex = compiled(pattern);
        if let Some(captures) = regex.captures(text) {
            if let Some(value) = captures.get(1) {
                if let Ok(parsed) = value.as_str().parse::<i64>() {
                    if parsed >= 1 {
                        return Some(parsed);
                    }
                }
            }
        }
    }
    None
}

/// Mapeia moedas ganhas no check-in para o dia da sequência (10→1 … 40→7).
#[must_use]
pub fn streak_from_checkin_coins(coins: Option<&str>) -> Option<i64> {
    let coins = coins?;
    let digits: String = coins.chars().filter(char::is_ascii_digit).collect();
    let value: i64 = digits.parse().ok()?;
    match value {
        10 => Some(1),
        15 => Some(2),
        20 => Some(3),
        25 => Some(4),
        30 => Some(5),
        35 => Some(6),
        40 => Some(7),
        _ => None,
    }
}

/// Moedas esperadas pelo dia da sequência (reusa o mapa do `core::report`).
#[must_use]
pub fn checkin_coins_from_streak(streak: Option<&ali_coins_core::report::StreakValue>) -> i64 {
    ali_coins_core::report::checkin_coins_from_streak(streak)
}

/// Extrai bônus de check-in e missões da seção de hoje do extrato.
#[must_use]
pub fn extract_today_ledger(today_section: &str) -> LedgerExtract {
    let entry_regex = compiled(r"([^\n+][^\n]*)\n\s*\+([0-9][0-9.,]*)");
    let label_regex = compiled(CHECKIN_LABEL_PATTERN);

    let mut bonus_coins: Option<i64> = None;
    let mut bonus_count = 0_u32;
    let mut missions_coins = 0_i64;
    let mut missions_count = 0_u32;

    for captures in entry_regex.captures_iter(today_section) {
        let label = captures.get(1).map_or("", |value| value.as_str()).trim();
        let raw_value = captures.get(2).map_or("", |value| value.as_str());
        let digits: String = raw_value.chars().filter(char::is_ascii_digit).collect();
        let Ok(value) = digits.parse::<i64>() else {
            continue;
        };
        if label.is_empty() {
            continue;
        }
        if label_regex.is_match(label) {
            bonus_count += 1;
            bonus_coins = Some(bonus_coins.unwrap_or(0) + value);
        } else {
            missions_count += 1;
            missions_coins += value;
        }
    }

    LedgerExtract {
        bonus_coins,
        missions_coins,
        bonus_count,
        missions_count,
    }
}

/// Extrai a sequência de check-ins consecutivos do histórico do extrato desktop.
///
/// `today_la` é a data de "hoje" no fuso `America/Los_Angeles` (injetável para testes);
/// o registro mais recente precisa ser de hoje ou ontem, senão `None`.
#[must_use]
pub fn get_streak_from_desktop_history(
    desktop_text: &str,
    today_la: chrono::NaiveDate,
) -> Option<i64> {
    if desktop_text.is_empty() {
        return None;
    }
    let block_regex = compiled(r"([0-9]{1,2})/([0-9]{1,2})/([0-9]{4})\s*PT");
    let label_regex = compiled(&format!(r"(?i){CHECKIN_LABEL_PATTERN}\s*\n\s*\+([0-9]+)"));

    let mut blocks: Vec<(i64, i64, i64, usize)> = Vec::new();
    for captures in block_regex.captures_iter(desktop_text) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        let day = captures
            .get(1)
            .and_then(|value| value.as_str().parse().ok());
        let month = captures
            .get(2)
            .and_then(|value| value.as_str().parse().ok());
        let year = captures
            .get(3)
            .and_then(|value| value.as_str().parse().ok());
        if let (Some(day), Some(month), Some(year)) = (day, month, year) {
            blocks.push((day, month, year, full.end()));
        }
    }
    if blocks.is_empty() {
        return None;
    }

    let mut raw_matches: Vec<(i64, i64, i64, i64)> = Vec::new();
    for (index, (day, month, year, start)) in blocks.iter().enumerate() {
        let end = blocks
            .get(index + 1)
            .map_or(desktop_text.len(), |next| next.3);
        let section = &desktop_text[*start..end];
        if let Some(captures) = label_regex.captures(section) {
            if let Some(coins) = captures
                .get(1)
                .and_then(|value| value.as_str().parse().ok())
            {
                raw_matches.push((*day, *month, *year, coins));
            }
        }
    }
    if raw_matches.is_empty() {
        return None;
    }

    let has_p1_gt12 = raw_matches.iter().any(|(day, ..)| *day > 12);
    let has_p2_gt12 = raw_matches.iter().any(|(_, month, ..)| *month > 12);

    let is_us_format = if has_p2_gt12 {
        true
    } else if has_p1_gt12 {
        false
    } else {
        let has_pt_header = compiled(r"(?i)Minhas moedas|B[ôo]nus di[áa]rio|Miss[õo]es de moedas")
            .is_match(desktop_text);
        let has_en_header = compiled(r"(?i)My coins").is_match(desktop_text);
        if has_pt_header && !has_en_header {
            false
        } else if has_en_header && !has_pt_header {
            true
        } else {
            let today_us = format!(
                "{:02}/{:02}/{} PT",
                today_la.month(),
                today_la.day(),
                today_la.year()
            );
            desktop_text.contains(&today_us)
        }
    };

    let mut entries: Vec<(chrono::NaiveDate, i64)> = Vec::new();
    for (p1, p2, year, coins) in raw_matches {
        let (day, month) = if is_us_format { (p2, p1) } else { (p1, p2) };
        let parsed = (
            i32::try_from(year).ok(),
            u32::try_from(month).ok(),
            u32::try_from(day).ok(),
        );
        let (Some(year), Some(month), Some(day)) = parsed else {
            continue;
        };
        let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) else {
            continue;
        };
        entries.push((date, coins));
    }

    let mut seen = std::collections::HashSet::new();
    let mut unique_days: Vec<i64> = Vec::new();
    for (date, _coins) in entries {
        let day_key = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp() / 86_400;
        if seen.insert(day_key) {
            unique_days.push(day_key);
        }
    }
    if unique_days.is_empty() {
        return None;
    }
    unique_days.sort_unstable_by(|a, b| b.cmp(a));

    let today_key = today_la.and_hms_opt(0, 0, 0)?.and_utc().timestamp() / 86_400;
    if unique_days[0] < today_key - 1 {
        return None;
    }

    let mut streak = 1_i64;
    for window in unique_days.windows(2) {
        if window[1] == window[0] - 1 {
            streak += 1;
        } else {
            break;
        }
    }
    Some(streak)
}

fn compiled(pattern: &str) -> &'static Regex {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static Regex>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache.lock().expect("cache de regex");
    if let Some(found) = cache.get(pattern) {
        return found;
    }
    let regex: &'static Regex = Box::leak(Box::new(Regex::new(pattern).expect("regex válida")));
    cache.insert(pattern.to_string(), regex);
    regex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_multilingue() {
        assert_eq!(extract_streak_from_text("Sequência de 42 dias"), Some(42));
        assert_eq!(
            extract_streak_from_text("You are on a 15-day streak"),
            Some(15)
        );
        assert_eq!(extract_streak_from_text("212 dias consecutivos"), Some(212));
        assert_eq!(extract_streak_from_text("Check-in diário: 7 dias"), Some(7));
        assert_eq!(extract_streak_from_text("coletou por 3 dias"), Some(3));
        assert_eq!(extract_streak_from_text("sem numero nenhum"), None);
    }

    #[test]
    fn prompt_de_login() {
        assert!(is_login_prompt_text("Sign in with email code"));
        assert!(is_login_prompt_text("Forgot password? Digite sua senha"));
        assert!(is_login_prompt_text("Trocar de conta"));
        assert!(!is_login_prompt_text("Minhas moedas 120"));
    }

    #[test]
    fn mapa_de_moedas() {
        assert_eq!(streak_from_checkin_coins(Some("+10 moedas")), Some(1));
        assert_eq!(streak_from_checkin_coins(Some("40")), Some(7));
        assert_eq!(streak_from_checkin_coins(Some("12")), None);
        assert_eq!(streak_from_checkin_coins(None), None);
    }

    #[test]
    fn extrato_de_hoje() {
        let text =
            "21/09/2026 App daily check-in\n+40\nCoin page task\n+5\nMissões de moedas\n+1.000";
        let extract = extract_today_ledger(text);
        assert_eq!(extract.bonus_coins, Some(40));
        assert_eq!(extract.bonus_count, 1);
        assert_eq!(extract.missions_coins, 1005);
        assert_eq!(extract.missions_count, 2);

        let today = "Bônus diário\n+20";
        let extract = extract_today_ledger(today);
        assert_eq!(extract.bonus_coins, Some(20));
        assert_eq!(extract.missions_coins, 0);
    }

    fn date(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(year, month, day).expect("data válida")
    }

    #[test]
    fn historico_consecutivo_formato_br() {
        let text = "Minhas moedas\n25/09/2026 PT App daily check-in\n+40\n24/09/2026 PT App daily check-in\n+35\n23/09/2026 PT App daily check-in\n+30";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            Some(3)
        );
    }

    #[test]
    fn historico_quebra_quando_dia_falta() {
        let text = "Minhas moedas\n25/09/2026 PT App daily check-in\n+40\n24/09/2026 PT App daily check-in\n+35\n22/09/2026 PT App daily check-in\n+25";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            Some(2)
        );
    }

    #[test]
    fn historico_antigo_retorna_none() {
        let text = "Minhas moedas\n15/09/2026 PT App daily check-in\n+40\n14/09/2026 PT App daily check-in\n+35";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            None
        );
    }

    #[test]
    fn historico_formato_us() {
        let text = "My coins\n09/25/2026 PT App daily check-in\n+40\n09/24/2026 PT App daily check-in\n+35";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            Some(2)
        );
    }

    #[test]
    fn historico_ontem_ainda_vale() {
        let text = "Minhas moedas\n24/09/2026 PT App daily check-in\n+35";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            Some(1)
        );
    }

    #[test]
    fn historico_nao_cruza_datas() {
        let text = "Minhas moedas\n25/09/2026 PT\n24/09/2026 PT App daily check-in\n+35";
        assert_eq!(
            get_streak_from_desktop_history(text, date(2026, 9, 25)),
            Some(1)
        );
    }
}
