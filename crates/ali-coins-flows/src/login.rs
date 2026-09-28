//! Seletores e login (equivalente a `libs/selectors.js` login + `libs/ui/login.js`).
//!
//! Listas de seletores em cascata (CSS específico primeiro), preenchimento com
//! eventos `input/change`, submissão por botão/Enter, **2FA fail-fast em modo
//! não-interativo** e validação de cookies de autenticação com retries.
//!
//! Divergências registradas: slider humanizado e seletores `:has-text`
//! completos entram na paridade com o site real (D-07).

use ali_coins_browser::driver::{BrowserError, Page};
use serde_json::Value;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Seletores de login do oráculo (subconjunto sem `:has-text`, que exige JS).
pub mod selectors {
    /// Campo de usuário.
    pub const USERNAME_INPUTS: [&str; 5] = [
        "#fm-login-id",
        "input[name=\"loginId\"]",
        "input.cosmos-input[type=\"email\"]",
        "input.cosmos-input[type=\"text\"]",
        "input[type=\"email\"][name*=\"login\" i]",
    ];
    /// Campo de senha.
    pub const PASSWORD_INPUTS: [&str; 2] = ["input[type=\"password\"]", "#fm-login-password"];
    /// Botão de continuar.
    pub const CONTINUE_BUTTONS: [&str; 1] = ["button.cosmos-btn-primary"];
    /// Botão de entrar.
    pub const SIGN_IN_BUTTONS: [&str; 3] = [
        "button.cosmos-btn-primary",
        "button[type=\"submit\"]",
        "#fm-login-submit",
    ];
    /// Campo de 2FA (específico; nunca genérico).
    pub const TWO_FACTOR_INPUTS: [&str; 4] = [
        "#fm-login-code",
        "input[name=\"checkCode\"]",
        "input[autocomplete=\"one-time-code\"]",
        "input.check-code-input[type=\"tel\"][maxlength=\"6\"]",
    ];
    /// Handle do slider anti-bot.
    pub const SLIDER_HANDLE: [&str; 2] = ["#nc_1_n1z", ".btn_slide"];
}

/// Cookies que comprovam autenticação.
pub const AUTH_COOKIES: [&str; 2] = ["xman_us_t", "login_aliyunid_ticket"];

/// Erros de login.
#[derive(Debug, Error)]
pub enum LoginError {
    /// 2FA solicitado em ambiente sem TTY (exit 5 no CLI).
    #[error(
        "Execução não-interativa detectada (sem TTY). O AliExpress solicitou verificação de código 2FA."
    )]
    TwoFactorRequiredNonInteractive,
    /// Desafio anti-bot detectado.
    #[error("Captcha solicitado pelo AliExpress durante o login.")]
    CaptchaChallenge,
    /// Fluxo interativo de 2FA ainda não suportado no port.
    #[error(
        "Leitura interativa de 2FA ainda não implementada no port (use export/import de sessão)."
    )]
    InteractiveTwoFactorUnsupported,
    /// Login não resultou em cookies válidos.
    #[error("Login não resultou em cookies de autenticação válidos após as tentativas.")]
    NoAuthCookies,
    /// Falha de browser propagada.
    #[error("{0}")]
    Browser(String),
}

impl From<BrowserError> for LoginError {
    fn from(error: BrowserError) -> Self {
        Self::Browser(error.to_string())
    }
}

/// Opções do fluxo de login.
#[derive(Debug, Clone)]
pub struct LoginOptions {
    /// Ambiente interativo (TTY) permite 2FA manual.
    pub interactive: bool,
    /// Tentativas de validação de cookie.
    pub cookie_attempts: u32,
    /// Espera entre validações de cookie.
    pub cookie_retry_delay: Duration,
    /// Timeout curto de detecção de cada seletor.
    pub detect_timeout: Duration,
    /// Espera máxima pelo campo de senha após o envio do usuário (SPA).
    pub password_wait_timeout: Duration,
}

impl Default for LoginOptions {
    fn default() -> Self {
        Self {
            interactive: false,
            cookie_attempts: 5,
            cookie_retry_delay: Duration::from_secs(1),
            detect_timeout: Duration::from_millis(300),
            password_wait_timeout: Duration::from_secs(15),
        }
    }
}

/// Resultado do login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginOutcome {
    /// Autenticado com cookies válidos.
    Success,
    /// 2FA necessário (modo não-interativo).
    TwoFactorRequired,
}

fn log_info(message: &str) {
    ali_coins_core::logging::global().info(message, &[]);
}

async fn first_present(page: &dyn Page, candidates: &[&str], timeout: Duration) -> Option<String> {
    for candidate in candidates {
        if page.wait_for_selector(candidate, timeout).await.is_ok() {
            return Some((*candidate).to_string());
        }
    }
    None
}

/// Clica no primeiro elemento VISÍVEL de cada seletor (ignora botões ocultos).
async fn click_visible(page: &dyn Page, candidates: &[&str]) -> Option<String> {
    for candidate in candidates {
        let script = format!(
            "(() => {{ const els = document.querySelectorAll({}); \
             for (const el of els) {{ const r = el.getBoundingClientRect(); \
             if (r.width > 0 && r.height > 0 && el.offsetParent !== null) {{ el.click(); return true; }} }} \
             return false; }})()",
            serde_json::to_string(candidate).unwrap_or_default()
        );
        let clicked = page
            .eval_raw(&script)
            .await
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if clicked {
            return Some((*candidate).to_string());
        }
    }
    None
}

/// Aguarda qualquer um dos seletores até o deadline, re-testando em ciclos curtos.
async fn wait_for_any(page: &dyn Page, candidates: &[&str], timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = first_present(page, candidates, Duration::from_millis(300)).await {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Preenche um input com eventos `input/change` (aceita React/Vue).
pub async fn fill_input(page: &dyn Page, selector: &str, value: &str) -> Result<(), LoginError> {
    let script = format!(
        "(() => {{ const el = document.querySelector({}); if (!el) return false; \
         const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype; \
         const setter = Object.getOwnPropertyDescriptor(proto, 'value').set; \
         setter.call(el, {}); \
         el.dispatchEvent(new Event('input', {{ bubbles: true }})); \
         el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
         return true; }})()",
        serde_json::to_string(selector).unwrap_or_default(),
        serde_json::to_string(value).unwrap_or_default()
    );
    page.eval_raw(&script).await?;
    Ok(())
}

/// Dispara `Enter` no elemento.
pub async fn press_enter(page: &dyn Page, selector: &str) -> Result<(), LoginError> {
    let script = format!(
        "(() => {{ const el = document.querySelector({}); if (!el) return false; \
         el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true }})); \
         el.dispatchEvent(new KeyboardEvent('keyup', {{ key: 'Enter', bubbles: true }})); \
         el.dispatchEvent(new KeyboardEvent('keypress', {{ key: 'Enter', bubbles: true }})); \
         return true; }})()",
        serde_json::to_string(selector).unwrap_or_default()
    );
    page.eval_raw(&script).await?;
    Ok(())
}

/// Há desafio anti-bot visível (slider) na página?
pub async fn has_captcha_challenge(page: &dyn Page, timeout: Duration) -> bool {
    first_present(page, &selectors::SLIDER_HANDLE, timeout)
        .await
        .is_some()
}

/// Valida os cookies de autenticação no storage state.
pub async fn has_auth_cookies(page: &dyn Page) -> Result<bool, LoginError> {
    let state: Value = page.storage_state().await?;
    let Some(cookies) = state.get("cookies").and_then(Value::as_array) else {
        return Ok(false);
    };
    Ok(cookies.iter().any(|cookie| {
        let name = cookie
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let value = cookie
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        AUTH_COOKIES.contains(&name) && !value.is_empty()
    }))
}

/// Executa o fluxo de login (SPA só-senha, usuário+senha e 2FA fail-fast).
pub async fn run_login(
    page: &dyn Page,
    user: &str,
    password: &str,
    options: &LoginOptions,
) -> Result<LoginOutcome, LoginError> {
    log_info("Iniciando fluxo de login.");
    // 1. Usuário (fluxo completo) — ausente no SPA que pede só senha.
    if let Some(username_selector) = wait_for_any(
        page,
        &selectors::USERNAME_INPUTS,
        options.password_wait_timeout,
    )
    .await
    {
        log_info(&format!("Campo de usuário detectado: {username_selector}"));
        fill_input(page, &username_selector, user).await?;
        press_enter(page, &username_selector).await?;
    }

    // 2. Senha (o SPA pode revelar o campo após um round-trip).
    let mut password_selector = wait_for_any(
        page,
        &selectors::PASSWORD_INPUTS,
        options.password_wait_timeout,
    )
    .await;
    if password_selector.is_none() {
        // Fallback do oráculo: botão Continue após o usuário.
        if let Some(button) =
            first_present(page, &selectors::CONTINUE_BUTTONS, options.detect_timeout).await
        {
            log_info(&format!("Clicando em continuar: {button}"));
            let _ = click_visible(page, &selectors::CONTINUE_BUTTONS).await;
        }
        password_selector = wait_for_any(
            page,
            &selectors::PASSWORD_INPUTS,
            options.password_wait_timeout,
        )
        .await;
    }
    if let Some(password_selector) = password_selector {
        log_info(&format!("Campo de senha detectado: {password_selector}"));
        fill_input(page, &password_selector, password).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Some(button) =
            wait_for_any(page, &selectors::SIGN_IN_BUTTONS, Duration::from_secs(5)).await
        {
            log_info(&format!("Submetendo login pelo botão: {button}"));
            let clicked = click_visible(page, &selectors::SIGN_IN_BUTTONS).await;
            log_info(&format!("Botão visível clicado: {clicked:?}"));
        } else {
            log_info("Submetendo login com Enter.");
            press_enter(page, &password_selector).await?;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        log_info(&format!(
            "URL após submit: {}",
            page.url().await.unwrap_or_default()
        ));
        if let Ok(value) = page
            .eval_raw("document.body ? document.body.innerText : ''")
            .await
        {
            if let Some(text) = value.as_str() {
                let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let snippet: String = collapsed.chars().take(300).collect();
                let sanitized = regex::Regex::new(r"[\w.+-]+@[\w.-]+")
                    .map(|re| re.replace_all(&snippet, "[email]").to_string())
                    .unwrap_or(snippet);
                log_info(&format!("Texto da página pós-submit: {sanitized}"));
            }
        }
    }

    // 3. 2FA (somente com input específico visível).
    if first_present(page, &selectors::TWO_FACTOR_INPUTS, options.detect_timeout)
        .await
        .is_some()
    {
        log_info("Campo de 2FA detectado.");
        return if options.interactive {
            Err(LoginError::InteractiveTwoFactorUnsupported)
        } else {
            Err(LoginError::TwoFactorRequiredNonInteractive)
        };
    }

    // 4. Validação dos cookies com retries.
    if let Ok(state) = page.storage_state().await {
        let count = state
            .get("cookies")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let names: Vec<&str> = state
            .get("cookies")
            .and_then(Value::as_array)
            .map(|cookies| {
                cookies
                    .iter()
                    .filter_map(|cookie| cookie.get("name").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        log_info(&format!(
            "Cookies presentes após submit: {count} ({})",
            names.join(", ")
        ));
    }
    for attempt in 0..options.cookie_attempts.max(1) {
        if has_auth_cookies(page).await? {
            return Ok(LoginOutcome::Success);
        }
        if attempt + 1 < options.cookie_attempts.max(1) {
            tokio::time::sleep(options.cookie_retry_delay).await;
        }
    }

    let captcha = has_captcha_challenge(page, options.detect_timeout).await;
    log_info(&format!("Desafio anti-bot visível após login: {captcha}"));
    if captcha {
        return Err(LoginError::CaptchaChallenge);
    }
    Err(LoginError::NoAuthCookies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockDriver, MockPageSpec};

    async fn page_with(spec: MockPageSpec) -> (MockDriver, Box<dyn Page>) {
        let driver = MockDriver::new(vec![spec]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        (driver, page)
    }

    fn auth_state() -> Value {
        serde_json::json!({
            "cookies": [{ "name": "xman_us_t", "value": "abc" }],
            "origins": []
        })
    }

    #[tokio::test]
    async fn login_feliz_com_usuario_e_senha() {
        let (_, page) = page_with(MockPageSpec {
            visible_selectors: vec![
                "#fm-login-id".to_string(),
                "input[type=\"password\"]".to_string(),
                "button[type=\"submit\"]".to_string(),
            ],
            storage_state: Some(auth_state()),
            ..MockPageSpec::default()
        })
        .await;
        let options = LoginOptions {
            cookie_attempts: 1,
            detect_timeout: Duration::from_millis(10),
            ..LoginOptions::default()
        };
        let outcome = run_login(&*page, "user@example.com", "senha", &options)
            .await
            .expect("login");
        assert_eq!(outcome, LoginOutcome::Success);
    }

    #[tokio::test]
    async fn spa_so_senha_nao_quebra() {
        let (_, page) = page_with(MockPageSpec {
            visible_selectors: vec!["input[type=\"password\"]".to_string()],
            storage_state: Some(auth_state()),
            ..MockPageSpec::default()
        })
        .await;
        let options = LoginOptions {
            cookie_attempts: 1,
            detect_timeout: Duration::from_millis(10),
            ..LoginOptions::default()
        };
        assert_eq!(
            run_login(&*page, "user@example.com", "senha", &options)
                .await
                .unwrap(),
            LoginOutcome::Success
        );
    }

    #[tokio::test]
    async fn dois_fatores_fail_fast() {
        let (_, page) = page_with(MockPageSpec {
            visible_selectors: vec!["#fm-login-code".to_string()],
            ..MockPageSpec::default()
        })
        .await;
        let options = LoginOptions {
            detect_timeout: Duration::from_millis(10),
            ..LoginOptions::default()
        };
        let error = run_login(&*page, "user@example.com", "senha", &options)
            .await
            .expect_err("2FA");
        assert!(matches!(error, LoginError::TwoFactorRequiredNonInteractive));
    }

    #[tokio::test]
    async fn captcha_vira_erro_tipado() {
        let (_, page) = page_with(MockPageSpec {
            visible_selectors: vec!["#nc_1_n1z".to_string()],
            storage_state: Some(serde_json::json!({ "cookies": [], "origins": [] })),
            ..MockPageSpec::default()
        })
        .await;
        let options = LoginOptions {
            cookie_attempts: 1,
            detect_timeout: Duration::from_millis(10),
            ..LoginOptions::default()
        };
        let error = run_login(&*page, "user@example.com", "senha", &options)
            .await
            .expect_err("captcha");
        assert!(matches!(error, LoginError::CaptchaChallenge));
    }

    #[tokio::test]
    async fn sem_cookies_esgota_tentativas() {
        let (_, page) = page_with(MockPageSpec {
            storage_state: Some(serde_json::json!({ "cookies": [], "origins": [] })),
            ..MockPageSpec::default()
        })
        .await;
        let options = LoginOptions {
            cookie_attempts: 2,
            cookie_retry_delay: Duration::from_millis(1),
            detect_timeout: Duration::from_millis(10),
            ..LoginOptions::default()
        };
        let error = run_login(&*page, "user@example.com", "senha", &options)
            .await
            .expect_err("sem auth");
        assert!(matches!(error, LoginError::NoAuthCookies));
    }
}
