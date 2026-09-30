//! Paridade fina do check-in (D-08): pré-checagem desktop, reuso, coleta de
//! água, sincronização de saldo e `resolveStreakDays` — port de trechos do
//! `collect.js`/`libs/ui/balance.js`.

use ali_coins_browser::driver::{Browser, Page};
use ali_coins_core::report::{
    ResolveStreakParams, ResolvedStreak, StreakValue, resolve_streak_days,
};
use ali_coins_core::{logging, time};
use ali_coins_flows::desktop::{DesktopBalance, read_desktop_report};
use std::time::Duration;

/// Pré-checagem desktop (`getBalanceDesktop` antes do mobile no oráculo).
pub async fn read_early_desktop(
    browser: &dyn Browser,
    storage_state: Option<&serde_json::Value>,
    timeout: Duration,
) -> Option<DesktopBalance> {
    read_desktop_report(browser, storage_state, timeout).await
}

/// Reutiliza a checagem inicial quando nada foi coletado e o saldo é válido
/// (port de `shouldReuseEarlyDesktop`).
#[must_use]
pub fn should_reuse_early_desktop(just_collected: bool, early: Option<&DesktopBalance>) -> bool {
    if just_collected {
        return false;
    }
    early.is_some_and(|data| {
        data.total_balance
            .as_deref()
            .is_some_and(|balance| !balance.trim().is_empty() && balance != "N/D")
    })
}

/// Coleta água da Fazenda Mágica se visível (trecho do `collect.js`).
pub async fn collect_water(page: &dyn Page) -> bool {
    let script = r#"(() => {
      const sels = ['.Footer--waterCollectedButtonBg--2jKL1c5', '[class*="waterCollected"]'];
      let el = null;
      for (const sel of sels) { const found = document.querySelector(sel); if (found) { el = found; break; } }
      if (!el) {
        el = Array.from(document.querySelectorAll('button')).find((b) => /regar|water/i.test(b.innerText || '')) || null;
      }
      if (!el) return false;
      el.click();
      return true;
    })()"#;
    let clicked = page
        .eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if clicked {
        logging::global().info("Coletando água da Fazenda Mágica...", &[]);
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    clicked
}

/// Sincroniza o saldo quando o ledger ainda não refletiu o crédito do
/// check-in (port do trecho "Saldo base das tarefas").
#[must_use]
pub fn sync_balance_after_checkin(
    just_collected: bool,
    checkin_coins: Option<i64>,
    balance: Option<&str>,
    early_balance: Option<&str>,
) -> Option<String> {
    let balance = balance?;
    if !just_collected {
        return Some(balance.to_string());
    }
    let Some(coins) = checkin_coins.filter(|value| *value > 0) else {
        return Some(balance.to_string());
    };
    let Some(current) = parse_digits(balance) else {
        return Some(balance.to_string());
    };
    let early = early_balance.and_then(parse_digits);
    let already_credited = early.is_some_and(|base| current >= base + coins);
    if already_credited {
        return Some(balance.to_string());
    }
    let base = early.unwrap_or(current);
    let updated = base + coins;
    logging::global().info(
        "Saldo pós-checkin sincronizado com as moedas recebidas no check-in (ledger defasado).",
        &[],
    );
    Some(updated.to_string())
}

/// Confirmação do check-in pelo crédito de hoje no extrato (port de
/// `shouldConfirmCheckinByLedger`).
#[must_use]
pub fn should_confirm_checkin_by_ledger(
    just_collected: bool,
    already_collected: bool,
    has_bonus_from_ledger: bool,
) -> bool {
    !just_collected && !already_collected && has_bonus_from_ledger
}

/// Precisa confirmar a quebra de streak pelo extrato? (port de
/// `shouldConfirmStreakByStatement`).
#[must_use]
pub fn should_confirm_streak_by_statement(
    detected_streak: Option<i64>,
    previous_streak_days: Option<i64>,
    statement_streak: Option<i64>,
    already_collected: bool,
) -> bool {
    if already_collected {
        return false;
    }
    if detected_streak != Some(1) {
        return false;
    }
    if previous_streak_days.is_none_or(|value| value <= 1) {
        return false;
    }
    // Extrato já disponível (confirma ou desmente) → não precisa de nova leitura.
    statement_streak.is_none()
}

/// `resolveStreakDays` com os mesmos parâmetros do oráculo.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn resolve_streak(
    detected_streak: Option<i64>,
    previous_streak_days: Option<i64>,
    early_desktop_streak: Option<i64>,
    just_collected: bool,
    already_collected: bool,
    confirmed_by_ledger: bool,
    statement_streak: Option<i64>,
) -> ResolvedStreak {
    let detected = detected_streak.map(StreakValue::Number);
    let statement = statement_streak.map(StreakValue::Number);
    resolve_streak_days(&ResolveStreakParams {
        detected_streak: detected.as_ref(),
        previous_streak_days,
        early_desktop_streak,
        just_collected,
        already_collected,
        confirmed_by_ledger,
        statement_streak: statement.as_ref(),
    })
}

/// Converte o streak resolvido para o número usado no restante do fluxo.
#[must_use]
pub fn resolved_streak_number(resolved: &ResolvedStreak) -> Option<i64> {
    match &resolved.streak_days {
        StreakValue::Number(value) => Some(*value),
        StreakValue::Text(text) => parse_digits(text),
    }
}

/// Formata a hora prevista de início do atraso inicial (`HH:mm:ss TZ`).
#[must_use]
pub fn format_target_time(target: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "{} {}",
        time::format_time(target),
        time::get_report_timezone_label(target)
    )
}

fn parse_digits(text: &str) -> Option<i64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    digits.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn early(balance: Option<&str>) -> DesktopBalance {
        DesktopBalance {
            total_balance: balance.map(str::to_string),
            ..DesktopBalance::default()
        }
    }

    #[test]
    fn reuso_da_checagem_inicial() {
        assert!(!should_reuse_early_desktop(true, Some(&early(Some("100")))));
        assert!(!should_reuse_early_desktop(false, None));
        assert!(!should_reuse_early_desktop(false, Some(&early(None))));
        assert!(!should_reuse_early_desktop(
            false,
            Some(&early(Some("N/D")))
        ));
        assert!(should_reuse_early_desktop(false, Some(&early(Some("100")))));
    }

    #[test]
    fn sincronizacao_de_saldo_quando_ledger_atrasa() {
        // Já creditado: mantém o saldo lido.
        assert_eq!(
            sync_balance_after_checkin(true, Some(40), Some("1140"), Some("1100")).as_deref(),
            Some("1140")
        );
        // Ledger atrasado: soma o crédito ao saldo anterior.
        assert_eq!(
            sync_balance_after_checkin(true, Some(40), Some("1100"), Some("1100")).as_deref(),
            Some("1140")
        );
        // Sem coleta nova: mantém.
        assert_eq!(
            sync_balance_after_checkin(false, Some(40), Some("1100"), Some("1060")).as_deref(),
            Some("1100")
        );
    }

    #[test]
    fn confirmacao_por_ledger() {
        assert!(should_confirm_checkin_by_ledger(false, false, true));
        assert!(!should_confirm_checkin_by_ledger(true, false, true));
        assert!(!should_confirm_checkin_by_ledger(false, true, true));
        assert!(!should_confirm_checkin_by_ledger(false, false, false));
    }

    #[test]
    fn confirma_quebra_de_streak_pelo_extrato() {
        assert!(should_confirm_streak_by_statement(
            Some(1),
            Some(226),
            None,
            false
        ));
        assert!(!should_confirm_streak_by_statement(
            Some(1),
            Some(226),
            Some(1),
            false
        ));
        assert!(!should_confirm_streak_by_statement(
            Some(1),
            Some(226),
            None,
            true
        ));
        assert!(!should_confirm_streak_by_statement(
            Some(2),
            Some(226),
            None,
            false
        ));
        assert!(!should_confirm_streak_by_statement(
            Some(1),
            Some(1),
            None,
            false
        ));
        assert!(!should_confirm_streak_by_statement(
            None,
            Some(226),
            None,
            false
        ));
    }

    #[test]
    fn streak_incrementa_com_a_base_anterior() {
        // Coleta nova com leitura móvel <= base: incrementa a base.
        let resolved = resolve_streak(Some(2), Some(2), None, true, false, false, None);
        assert_eq!(resolved_streak_number(&resolved), Some(3));

        // Reexecução (já coletado): preserva a base.
        let resolved = resolve_streak(Some(2), Some(2), None, false, true, false, None);
        assert_eq!(resolved_streak_number(&resolved), Some(2));
    }
}
