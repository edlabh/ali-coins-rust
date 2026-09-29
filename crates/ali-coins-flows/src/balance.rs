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

/// Saldo total no texto visível do desktop (`My coins`/`Minhas moedas`).
///
/// Equivale às regexes de `libs/ui/balance.js::getBalanceDesktop`: o número
/// fica na linha seguinte ao rótulo (bilíngue, com separador de milhar), com
/// fallback para o padrão `<número>\n<saves>`. Devolve só os dígitos.
#[must_use]
pub fn parse_desktop_balance(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let labeled = compiled(r"(?i)(?:My coins|Minhas moedas)\s*\n\s*([0-9][0-9.,]*)");
    let saves = compiled(r"(?i)([0-9][0-9.,]*)\s*\n\s*saves");
    let captured = labeled.captures(text).or_else(|| saves.captures(text))?;
    let digits: String = captured
        .get(1)?
        .as_str()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then_some(digits)
}

/// Seção do extrato pertencente a `today_pt` (as datas do mycoin são PT).
///
/// Aceita a data em pt-BR (`DD/MM/AAAA PT`) e en-US (`M/D/AAAA PT`); sem
/// correspondência direta, compara a data normalizada de cada bloco — mesma
/// estratégia de `getBalanceDesktop`.
#[must_use]
pub fn extract_today_section(text: &str, today_pt: chrono::NaiveDate) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let pt_br = format!(
        "{:02}/{:02}/{} PT",
        today_pt.day(),
        today_pt.month(),
        today_pt.year()
    );
    let pt_us = format!(
        "{}/{}/{} PT",
        today_pt.month(),
        today_pt.day(),
        today_pt.year()
    );
    let date_re = compiled(r"[0-9]{1,2}/[0-9]{1,2}/[0-9]{4}\s*PT");

    if let Some(section) = section_after(text, &pt_br, date_re) {
        return Some(section);
    }
    if let Some(section) = section_after(text, &pt_us, date_re) {
        return Some(section);
    }

    // Fallback: qualquer grafia cuja data normalizada seja a de hoje.
    let today = (today_pt.day(), today_pt.month(), today_pt.year());
    for found in date_re.find_iter(text) {
        if normalize_pt_date(found.as_str()) == Some(today) {
            let rest = &text[found.end()..];
            let end = date_re.find(rest).map_or(rest.len(), |next| next.start());
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Sequência no desktop: texto explícito > máximo (histórico, tier do bônus).
#[must_use]
pub fn desktop_streak(
    text: &str,
    today_pt: chrono::NaiveDate,
    today_checkin_coins: Option<&str>,
) -> Option<i64> {
    if let Some(found) = extract_streak_from_text(text) {
        return Some(found);
    }
    let history = get_streak_from_desktop_history(text, today_pt).unwrap_or(0);
    let tier = streak_from_checkin_coins(today_checkin_coins).unwrap_or(0);
    let calculated = history.max(tier);
    (calculated > 0).then_some(calculated)
}

/// Texto após o primeiro `needle`, limitado à próxima data PT (se houver).
fn section_after(text: &str, needle: &str, date_re: &regex::Regex) -> Option<String> {
    let (_, rest) = text.split_once(needle)?;
    let end = date_re.find(rest).map_or(rest.len(), |next| next.start());
    Some(rest[..end].to_string())
}

/// `D/M/AAAA PT` → `(dia, mês, ano)` com zeros à esquerda tolerados.
fn normalize_pt_date(raw: &str) -> Option<(u32, u32, i32)> {
    let trimmed = raw.trim();
    let clean = trimmed.strip_suffix("PT").unwrap_or(trimmed).trim();
    let mut parts = clean.split('/');
    let day = parts.next()?.trim().parse::<u32>().ok()?;
    let month = parts.next()?.trim().parse::<u32>().ok()?;
    let year = parts.next()?.trim().parse::<i32>().ok()?;
    Some((day, month, year))
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

    #[test]
    fn saldo_do_desktop_bilingue() {
        assert_eq!(
            parse_desktop_balance("Minhas moedas\n2.980\nMissões de moedas"),
            Some("2980".to_string())
        );
        assert_eq!(
            parse_desktop_balance("My coins\n1,234\nsaves"),
            Some("1234".to_string())
        );
        assert_eq!(
            parse_desktop_balance("2.980\nsaves"),
            Some("2980".to_string())
        );
        assert_eq!(parse_desktop_balance("sem saldo"), None);
        assert_eq!(parse_desktop_balance(""), None);
    }

    #[test]
    fn secao_de_hoje_pt_e_us() {
        let today = date(2026, 9, 28);
        let pt = "Minhas moedas\n2.980\n28/09/2026 PT\nBônus diário\n+20\nCoin page task\n+5\n27/09/2026 PT\nBônus diário\n+15";
        let section = extract_today_section(pt, today).expect("seção de hoje");
        assert!(section.contains("Bônus diário\n+20"));
        assert!(section.contains("Coin page task\n+5"));
        assert!(!section.contains("+15"));

        let us = "My coins\n1,234\n9/28/2026 PT\nDaily bonus\n+20\n9/27/2026 PT\nDaily bonus\n+15";
        let section = extract_today_section(us, today).expect("seção de hoje (en)");
        assert!(section.contains("Daily bonus\n+20"));
        assert!(!section.contains("+15"));

        // Grafia com zero à esquerda no formato en-US cai no fallback normalizado.
        let padded =
            "My coins\n1,234\n09/28/2026 PT\nDaily bonus\n+20\n09/27/2026 PT\nDaily bonus\n+15";
        let section = extract_today_section(padded, today).expect("seção de hoje (fallback)");
        assert!(section.contains("Daily bonus\n+20"));
        assert!(!section.contains("+15"));

        assert!(
            extract_today_section("My coins\n1,234\n9/27/2026 PT\nDaily bonus\n+15", today)
                .is_none()
        );
    }

    #[test]
    fn ledger_de_hoje_classifica_bonus_e_missoes() {
        let today = date(2026, 9, 28);
        let text = "My coins\n2,980\n28/09/2026 PT\nBônus diário\n+1\nCoin page task\n+56\n26/09/2026 PT\nBônus diário\n+40";
        let section = extract_today_section(text, today).expect("seção de hoje");
        let ledger = extract_today_ledger(&section);
        assert_eq!(ledger.bonus_coins, Some(1));
        assert_eq!(ledger.missions_coins, 56);
        assert_eq!(ledger.missions_count, 1);
    }

    #[test]
    fn streak_do_desktop_prefere_texto_e_cai_para_calculo() {
        let today = date(2026, 9, 28);
        let explicit =
            "Minhas moedas\n2.980\nSequência de 12 dias\n28/09/2026 PT\nBônus diário\n+20";
        assert_eq!(desktop_streak(explicit, today, Some("20")), Some(12));

        // Histórico de 2 dias, mas bônus +20 (tier 3): o oráculo usa max(histórico, tier).
        let history = "Minhas moedas\n2.980\n28/09/2026 PT App daily check-in\n+20\n27/09/2026 PT App daily check-in\n+15";
        assert_eq!(desktop_streak(history, today, Some("20")), Some(3));

        // Histórico de 3 dias supera o tier (+15 → dia 2).
        let history3 = "Minhas moedas\n2.980\n28/09/2026 PT App daily check-in\n+15\n27/09/2026 PT App daily check-in\n+15\n26/09/2026 PT App daily check-in\n+15";
        assert_eq!(desktop_streak(history3, today, Some("15")), Some(3));

        let tier_only = "Minhas moedas\n2.980\nBônus diário\n+40";
        assert_eq!(desktop_streak(tier_only, today, Some("40")), Some(7));

        assert_eq!(desktop_streak("Minhas moedas\n2.980", today, None), None);
    }
}
