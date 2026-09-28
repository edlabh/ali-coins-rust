//! Parsers de saldo/streak/extrato compatíveis com `libs/ui/balance.js`.
//!
//! Funções puras (sem browser): regexes multilíngues de streak, mapa
//! moedas⇄dias do ciclo oficial, detecção de prompt de login e extração do
//! extrato de hoje (bônus de check-in vs. missões).

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

fn compiled(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache.lock().expect("cache de regex");
    if let Some(found) = cache.get(pattern) {
        return found;
    }
    let regex: &'static Regex = Box::leak(Box::new(Regex::new(pattern).expect("regex válida")));
    cache.insert(pattern, regex);
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
}
