//! Driver de browser em memória para testes (sem Chromium).
//!
//! Reproduz a interface `BrowserDriver` com páginas roteirizadas: URLs,
//! conteúdos, respostas de evaluate, textos por seletor e registro de ações —
//! suficiente para portar os testes de fluxo e de helpers de UI.

use super::driver::{Browser, BrowserDriver, BrowserError, LaunchOptions, NavOptions, Page};
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Especificação de uma página roteirizada.
#[derive(Debug, Clone, Default)]
pub struct MockPageSpec {
    /// URL inicial.
    pub url: String,
    /// Título.
    pub title: String,
    /// HTML.
    pub content: String,
    /// Respostas de `eval_raw` por script exato.
    pub eval_responses: HashMap<String, Value>,
    /// Textos por seletor (`query_all_text`).
    pub selector_texts: HashMap<String, Vec<String>>,
    /// Seletores considerados visíveis.
    pub visible_selectors: Vec<String>,
    /// Storage state devolvido por `storage_state`.
    pub storage_state: Option<Value>,
    /// Avaliações por substring do script (fallback do map exato).
    pub eval_contains: Vec<(String, Value)>,
}

/// Ações registradas (para asserções de fluxo).
#[derive(Debug, Clone, PartialEq)]
pub enum MockAction {
    /// Navegação.
    Goto(String),
    /// Voltar.
    GoBack,
    /// Evaluate de script.
    Eval(String),
    /// Clique por seletor.
    Click(String),
    /// Scroll.
    Scroll(i64, i64),
    /// Screenshot.
    Screenshot,
}

#[derive(Default)]
struct MockState {
    url: String,
}

/// Driver em memória.
#[derive(Debug, Clone, Default)]
pub struct MockDriver {
    /// Páginas que serão criadas em sequência (cicla na última).
    pub pages: Vec<MockPageSpec>,
    /// Ações registradas, compartilhadas com o teste.
    pub actions: Arc<Mutex<Vec<MockAction>>>,
}

/// Browser mockado.
pub struct MockBrowser {
    pages: Vec<MockPageSpec>,
    actions: Arc<Mutex<Vec<MockAction>>>,
    opened: Arc<Mutex<usize>>,
}

/// Página mockada.
pub struct MockPage {
    spec: MockPageSpec,
    state: Arc<Mutex<MockState>>,
    actions: Arc<Mutex<Vec<MockAction>>>,
    closed: Arc<Mutex<bool>>,
}

impl MockDriver {
    /// Cria um driver com páginas roteirizadas.
    #[must_use]
    pub fn new(pages: Vec<MockPageSpec>) -> Self {
        Self {
            pages,
            actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Ações registradas.
    #[must_use]
    pub fn actions(&self) -> Vec<MockAction> {
        self.actions.lock().expect("ações").clone()
    }
}

#[async_trait]
impl BrowserDriver for MockDriver {
    async fn launch(&self, _options: &LaunchOptions) -> Result<Box<dyn Browser>, BrowserError> {
        Ok(Box::new(MockBrowser {
            pages: self.pages.clone(),
            actions: Arc::clone(&self.actions),
            opened: Arc::new(Mutex::new(0)),
        }))
    }
}

#[async_trait]
impl Browser for MockBrowser {
    async fn new_page(&self) -> Result<Box<dyn Page>, BrowserError> {
        let mut opened = self.opened.lock().expect("abertas");
        let index = if self.pages.is_empty() {
            0
        } else {
            (*opened).min(self.pages.len() - 1)
        };
        let spec = self.pages.get(index).cloned().unwrap_or_default();
        *opened += 1;
        Ok(Box::new(MockPage {
            state: Arc::new(Mutex::new(MockState {
                url: spec.url.clone(),
            })),
            spec,
            actions: Arc::clone(&self.actions),
            closed: Arc::new(Mutex::new(false)),
        }))
    }

    async fn pages(&self) -> Result<Vec<Box<dyn Page>>, BrowserError> {
        let page = self.new_page().await?;
        Ok(vec![page])
    }

    async fn version(&self) -> Result<String, BrowserError> {
        Ok("MockBrowser/1.0".to_string())
    }

    async fn close(self: Box<Self>) -> Result<(), BrowserError> {
        Ok(())
    }
}

impl MockPage {
    fn record(&self, action: MockAction) {
        self.actions.lock().expect("ações").push(action);
    }

    fn ensure_open(&self) -> Result<(), BrowserError> {
        if *self.closed.lock().expect("fechada") {
            return Err(BrowserError::PageClosed("mock".to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl Page for MockPage {
    async fn goto(&self, url: &str, _options: &NavOptions) -> Result<(), BrowserError> {
        self.ensure_open()?;
        self.state.lock().expect("estado").url = url.to_string();
        self.record(MockAction::Goto(url.to_string()));
        Ok(())
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.ensure_open()?;
        self.record(MockAction::GoBack);
        Ok(())
    }

    async fn url(&self) -> Result<String, BrowserError> {
        self.ensure_open()?;
        Ok(self.state.lock().expect("estado").url.clone())
    }

    async fn title(&self) -> Result<String, BrowserError> {
        Ok(self.spec.title.clone())
    }

    async fn content(&self) -> Result<String, BrowserError> {
        Ok(self.spec.content.clone())
    }

    async fn eval_raw(&self, script: &str) -> Result<Value, BrowserError> {
        self.ensure_open()?;
        self.record(MockAction::Eval(script.to_string()));
        if let Some(response) = self.spec.eval_responses.get(script) {
            return Ok(response.clone());
        }
        for (needle, response) in &self.spec.eval_contains {
            if script.contains(needle) {
                return Ok(response.clone());
            }
        }
        Ok(Value::Null)
    }

    async fn wait_for_selector(
        &self,
        selector: &str,
        _timeout: Duration,
    ) -> Result<(), BrowserError> {
        self.ensure_open()?;
        if self
            .spec
            .visible_selectors
            .iter()
            .any(|item| item == selector)
            || self.spec.selector_texts.contains_key(selector)
        {
            return Ok(());
        }
        Err(BrowserError::NotFound(selector.to_string()))
    }

    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        self.ensure_open()?;
        Ok(self
            .spec
            .selector_texts
            .get(selector)
            .cloned()
            .unwrap_or_default())
    }

    async fn click_selector(&self, selector: &str) -> Result<(), BrowserError> {
        self.ensure_open()?;
        self.record(MockAction::Click(selector.to_string()));
        Ok(())
    }

    async fn scroll_by(&self, x: i64, y: i64) -> Result<(), BrowserError> {
        self.ensure_open()?;
        self.record(MockAction::Scroll(x, y));
        Ok(())
    }

    async fn storage_state(&self) -> Result<Value, BrowserError> {
        self.ensure_open()?;
        Ok(self
            .spec
            .storage_state
            .clone()
            .unwrap_or_else(|| serde_json::json!({ "cookies": [], "origins": [] })))
    }

    async fn seed_storage_state(&self, _state: &Value) -> Result<(), BrowserError> {
        self.ensure_open()?;
        Ok(())
    }

    async fn enable_resource_blocking(&self, _allow_media: bool) -> Result<(), BrowserError> {
        self.ensure_open()?;
        Ok(())
    }

    async fn set_device_profile(
        &self,
        _profile: &super::launch::DeviceProfile,
    ) -> Result<(), BrowserError> {
        self.ensure_open()?;
        Ok(())
    }

    async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
        self.ensure_open()?;
        self.record(MockAction::Screenshot);
        Ok(b"PNG-mock".to_vec())
    }

    async fn close(&self) -> Result<(), BrowserError> {
        *self.closed.lock().expect("fechada") = true;
        Ok(())
    }
}

/// Helper: resposta de eval como JSON.
#[must_use]
pub fn json(value: Value) -> Value {
    value
}

/// Helper: mapa de respostas de eval.
#[must_use]
pub fn eval_map(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
    entries
        .into_iter()
        .map(|(script, value)| (script.to_string(), value))
        .collect()
}

/// Helper: Map vazio para specs.
#[must_use]
pub fn empty_map() -> Map<String, Value> {
    Map::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn driver_mockado_roteiriza_paginas() {
        let driver = MockDriver::new(vec![MockPageSpec {
            url: "https://m.aliexpress.com/p/coin-index/index.html".to_string(),
            title: "Moedas".to_string(),
            content: "<html></html>".to_string(),
            eval_responses: eval_map(vec![("document.title", json!("Moedas"))]),
            selector_texts: HashMap::from([(
                ".card".to_string(),
                vec!["A".to_string(), "B".to_string()],
            )]),
            visible_selectors: vec![".card".to_string()],
            ..MockPageSpec::default()
        }]);

        let browser = driver
            .launch(&LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("page");
        page.goto("https://example.com", &NavOptions::default())
            .await
            .expect("goto");
        assert_eq!(page.url().await.unwrap(), "https://example.com");
        assert_eq!(page.title().await.unwrap(), "Moedas");
        let title: String = super::super::driver::eval_as(&*page, "document.title")
            .await
            .unwrap();
        assert_eq!(title, "Moedas");
        page.wait_for_selector(".card", Duration::from_secs(1))
            .await
            .expect("visível");
        assert_eq!(
            page.query_all_text(".card").await.unwrap(),
            vec!["A".to_string(), "B".to_string()]
        );
        assert!(
            page.wait_for_selector(".ausente", Duration::from_secs(1))
                .await
                .is_err()
        );

        let actions = driver.actions();
        assert!(actions.contains(&MockAction::Goto("https://example.com".to_string())));
        assert!(actions.contains(&MockAction::Eval("document.title".to_string())));
    }

    #[tokio::test]
    async fn pagina_fechada_gera_erro() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        page.close().await.unwrap();
        assert!(matches!(page.url().await, Err(BrowserError::PageClosed(_))));
    }
}
