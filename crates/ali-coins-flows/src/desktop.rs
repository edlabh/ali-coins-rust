//! Leitura de saldo/extrato na página desktop (mycoin), equivalente a
//! `libs/ui/balance.js::getBalanceDesktop` do oráculo.
//!
//! O mobile confirma a coleta, mas saldo, valor real do bônus e ganhos das
//! tarefas vêm do extrato desktop — fonte de verdade `all.js`/`do_tasks.js`.

use crate::balance::{
    desktop_streak, extract_today_ledger, extract_today_section, is_login_prompt_text,
    parse_desktop_balance,
};
use ali_coins_browser::driver::{Browser, NavOptions, Page};
use ali_coins_core::logging;
use std::time::{Duration, Instant};

/// URL do extrato/saldo no desktop (página de moedas para PC).
pub const DESKTOP_MYCOIN_URL: &str = "https://www.aliexpress.com/p/coin-pc-index/mycoin.html";

/// Marcadores de que o ledger desktop renderizou (pt/en).
const DESKTOP_MARKERS: [&str; 6] = [
    "Minhas moedas",
    "My coins",
    "saves",
    "App daily check-in",
    "Bônus diário",
    "Daily bonus",
];

/// Resultado da leitura desktop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopBalance {
    /// Saldo total (só dígitos, ex.: `2980`).
    pub total_balance: Option<String>,
    /// Crédito real de check-in no extrato de hoje (`Bônus diário`/legado).
    pub today_bonus_coins: Option<i64>,
    /// Soma real dos ganhos de tarefas no extrato de hoje.
    pub today_missions_coins: Option<i64>,
    /// Quantidade de lançamentos de tarefas no dia.
    pub today_missions_count: u32,
    /// Extrato de hoje confirmou crédito de check-in.
    pub has_checkin_today: bool,
    /// Sequência lida/calculada no desktop (fallback do mobile).
    pub desktop_streak: Option<i64>,
    /// A leitura caiu num prompt de login/reautenticação.
    pub login_prompt_detected: bool,
    /// A seção de hoje do extrato foi localizada (ganhos são confiáveis).
    pub ledger_available: bool,
}

/// Abre uma página dedicada (sem emulação mobile), lê o desktop e fecha a página.
///
/// Best-effort: qualquer falha vira `None` e é logada — o fluxo principal segue.
pub async fn read_desktop_report(
    browser: &dyn Browser,
    storage_state: Option<&serde_json::Value>,
    timeout: Duration,
) -> Option<DesktopBalance> {
    let page = match browser.new_page().await {
        Ok(page) => page,
        Err(error) => {
            logging::global().warn(
                &format!("Falha ao abrir a página desktop de saldo: {error}"),
                &[],
            );
            return None;
        }
    };
    if let Some(state) = storage_state {
        let _ = page.seed_storage_state(state).await;
    }
    let balance = read_desktop_balance(&*page, timeout).await;
    logging::global().info(
        &format!(
            "Extrato desktop: saldo={} bônus={:?} missões={:?} streak={:?} (ledger disponível: {})",
            balance.total_balance.as_deref().unwrap_or("N/D"),
            balance.today_bonus_coins,
            balance.today_missions_coins,
            balance.desktop_streak,
            balance.ledger_available
        ),
        &[],
    );
    let _ = page.close().await;
    Some(balance)
}

/// Lê saldo, extrato do dia e sequência na página atual do desktop.
///
/// Erros de navegação são tolerados (o oráculo engole o `goto`); o parsing do
/// texto disponível decide o resultado.
pub async fn read_desktop_balance(page: &dyn Page, timeout: Duration) -> DesktopBalance {
    let _ = page
        .goto(
            DESKTOP_MYCOIN_URL,
            &NavOptions {
                timeout: Some(timeout),
                wait_until: Some("domcontentloaded".to_string()),
            },
        )
        .await;

    let text = wait_for_desktop_text(page, timeout).await;
    let today_pt = chrono::Utc::now()
        .with_timezone(&chrono_tz::America::Los_Angeles)
        .date_naive();

    let total_balance = parse_desktop_balance(&text);
    let has_marker = DESKTOP_MARKERS.iter().any(|marker| text.contains(marker));
    let login_prompt_text = is_login_prompt_text(&text);
    let login_prompt_detected = if total_balance.is_none() && !has_marker {
        login_prompt_text
            || page
                .wait_for_selector("input[type=\"password\"]", Duration::from_millis(1200))
                .await
                .is_ok()
    } else {
        login_prompt_text
    };

    let today_section = extract_today_section(&text, today_pt);
    let ledger = today_section.as_deref().map(extract_today_ledger);
    let today_bonus_coins = ledger.as_ref().and_then(|data| data.bonus_coins);
    let today_missions_coins = ledger.as_ref().map(|data| data.missions_coins);
    let today_missions_count = ledger.as_ref().map_or(0, |data| data.missions_count);
    let has_app_checkin_today = today_section.as_deref().is_some_and(|section| {
        section.contains("App daily check-in") || section.contains("Check-in diário no app")
    });
    let bonus_text = today_bonus_coins.map(|value| value.to_string());
    let desktop_streak = desktop_streak(&text, today_pt, bonus_text.as_deref());

    if total_balance.is_none() {
        logging::global().warn(
            "Saldo do desktop não detectado (layout mudou ou sessão inválida?).",
            &[],
        );
    }
    if login_prompt_detected {
        logging::global().warn(
            "[Sessão] Prompt de login/reautenticação detectado no desktop; saldo/ledger indisponíveis.",
            &[],
        );
    }

    DesktopBalance {
        total_balance,
        today_bonus_coins,
        today_missions_coins,
        today_missions_count,
        has_checkin_today: has_app_checkin_today || today_bonus_coins.is_some(),
        desktop_streak,
        login_prompt_detected,
        ledger_available: ledger.is_some(),
    }
}

/// Aguarda o texto do body até renderizar o ledger (ou o timeout).
async fn wait_for_desktop_text(page: &dyn Page, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    loop {
        if let Ok(value) = page
            .eval_raw("document.body ? document.body.innerText : ''")
            .await
        {
            if let Some(text) = value.as_str() {
                last = text.to_string();
            }
        }
        let rendered = DESKTOP_MARKERS.iter().any(|marker| last.contains(marker));
        if !last.is_empty() && (rendered || is_login_prompt_text(&last)) {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockDriver, MockPageSpec};
    use serde_json::json;
    use std::time::Duration;

    fn desktop_text() -> String {
        let today = chrono::Utc::now()
            .with_timezone(&chrono_tz::America::Los_Angeles)
            .date_naive();
        let today_br = format!(
            "{:02}/{:02}/{} PT",
            chrono::Datelike::day(&today),
            chrono::Datelike::month(&today),
            chrono::Datelike::year(&today)
        );
        format!(
            "Minhas moedas\n2.980\n{today_br}\nBônus diário\n+1\nCoin page task\n+56\nSequência de 3 dias"
        )
    }

    #[tokio::test]
    async fn leitura_desktop_completa() {
        let driver = MockDriver::new(vec![MockPageSpec {
            eval_contains: vec![(
                "document.body ? document.body.innerText".to_string(),
                json!(desktop_text()),
            )],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();

        let balance = read_desktop_balance(&*page, Duration::from_millis(50)).await;
        assert_eq!(balance.total_balance.as_deref(), Some("2980"));
        assert_eq!(balance.today_bonus_coins, Some(1));
        assert_eq!(balance.today_missions_coins, Some(56));
        assert_eq!(balance.today_missions_count, 1);
        assert!(balance.has_checkin_today);
        assert!(balance.ledger_available);
        assert_eq!(balance.desktop_streak, Some(3));
        assert!(!balance.login_prompt_detected);
    }

    #[tokio::test]
    async fn leitura_desktop_sem_ledger() {
        let driver = MockDriver::new(vec![MockPageSpec {
            eval_contains: vec![(
                "document.body ? document.body.innerText".to_string(),
                json!("Minhas moedas\n2.980"),
            )],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();

        let balance = read_desktop_balance(&*page, Duration::from_millis(50)).await;
        assert_eq!(balance.total_balance.as_deref(), Some("2980"));
        assert!(!balance.ledger_available);
        assert_eq!(balance.today_missions_coins, None);
    }

    #[tokio::test]
    async fn leitura_desktop_com_prompt_de_login() {
        let driver = MockDriver::new(vec![MockPageSpec {
            eval_contains: vec![(
                "document.body ? document.body.innerText".to_string(),
                json!("Sign in with email code\nEsqueci minha senha"),
            )],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();

        let balance = read_desktop_balance(&*page, Duration::from_millis(50)).await;
        assert!(balance.login_prompt_detected);
        assert_eq!(balance.total_balance, None);
    }

    #[tokio::test]
    async fn relatorio_desktop_cria_e_fecha_pagina() {
        let driver = MockDriver::new(vec![MockPageSpec {
            eval_contains: vec![(
                "document.body ? document.body.innerText".to_string(),
                json!(desktop_text()),
            )],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();

        let balance = read_desktop_report(&*browser, None, Duration::from_millis(50))
            .await
            .expect("leitura");
        assert_eq!(balance.total_balance.as_deref(), Some("2980"));
    }
}
