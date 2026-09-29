//! Fronteira do browser: trait de driver + tipos de opções (porta/adaptador).
//!
//! Os fluxos dependem apenas destes traits; `CdpDriver` (produção) e
//! `MockDriver` (testes) ficam atrás da mesma interface, permitindo portar a
//! suíte de testes sem Chromium e trocar a implementação sem tocar em regras.

use super::launch::DeviceProfile;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

/// Erros da camada de browser.
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    /// Falha ao iniciar/conectar no Chromium.
    #[error("falha ao iniciar o browser: {0}")]
    Launch(String),
    /// Falha de navegação.
    #[error("falha de navegação: {0}")]
    Navigation(String),
    /// Timeout aguardando seletor/condição.
    #[error("timeout aguardando: {0}")]
    Timeout(String),
    /// Erro de avaliação de script.
    #[error("falha no evaluate: {0}")]
    Evaluate(String),
    /// Elemento/seletor ausente.
    #[error("elemento não encontrado: {0}")]
    NotFound(String),
    /// Página/frame fechado durante a operação.
    #[error("página fechada: {0}")]
    PageClosed(String),
    /// Falha de I/O de diagnóstico.
    #[error("I/O: {0}")]
    Io(String),
    /// Recurso não suportado na implementação.
    #[error("não suportado: {0}")]
    Unsupported(String),
}

/// Opções de launch.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Modo headless.
    pub headless: bool,
    /// Args adicionais/do oráculo.
    pub args: Vec<String>,
    /// Ambiente sanitizado repassado ao processo.
    pub env: Vec<(String, String)>,
    /// Executável customizado.
    pub executable_path: Option<PathBuf>,
    /// Diretório de perfil (quando aplicável).
    pub user_data_dir: Option<PathBuf>,
}

/// Opções de navegação.
#[derive(Debug, Clone, Default)]
pub struct NavOptions {
    /// Timeout da navegação.
    pub timeout: Option<Duration>,
    /// Aguardar a página "carregar" (domcontentloaded/load).
    pub wait_until: Option<String>,
}

/// Driver de browser.
#[async_trait]
pub trait BrowserDriver: Send + Sync {
    /// Inicia um browser novo.
    async fn launch(&self, options: &LaunchOptions) -> Result<Box<dyn Browser>, BrowserError>;
}

/// Browser iniciado.
#[async_trait]
pub trait Browser: Send + Sync {
    /// Nova página (aba).
    async fn new_page(&self) -> Result<Box<dyn Page>, BrowserError>;
    /// Páginas abertas.
    async fn pages(&self) -> Result<Vec<Box<dyn Page>>, BrowserError>;
    /// Hostname/versão reportados (diagnóstico).
    async fn version(&self) -> Result<String, BrowserError>;
    /// Fecha o browser.
    async fn close(self: Box<Self>) -> Result<(), BrowserError>;
}

/// Avalia um script e desserializa o resultado no tipo pedido.
pub async fn eval_as<T: DeserializeOwned>(
    page: &dyn Page,
    script: &str,
) -> Result<T, BrowserError> {
    let value = page.eval_raw(script).await?;
    serde_json::from_value(value).map_err(|err| BrowserError::Evaluate(err.to_string()))
}

/// Página/aba.
#[async_trait]
pub trait Page: Send + Sync {
    /// Navega para a URL.
    async fn goto(&self, url: &str, options: &NavOptions) -> Result<(), BrowserError>;
    /// Volta no histórico (depende de `BackForwardCache` preservado).
    async fn go_back(&self) -> Result<(), BrowserError>;
    /// URL atual.
    async fn url(&self) -> Result<String, BrowserError>;
    /// Título atual.
    async fn title(&self) -> Result<String, BrowserError>;
    /// HTML da página.
    async fn content(&self) -> Result<String, BrowserError>;
    /// Avalia script devolvendo JSON cru (use [`eval_as`] para tipar).
    async fn eval_raw(&self, script: &str) -> Result<Value, BrowserError>;
    /// Habilita o bloqueio de recursos (imagem/mídia/fonte/telemetria).
    async fn enable_resource_blocking(&self, allow_media: bool) -> Result<(), BrowserError> {
        let _ = allow_media;
        Err(BrowserError::Unsupported(
            "enable_resource_blocking não suportado nesta implementação".to_string(),
        ))
    }

    /// Lê o storage state (cookies + localStorage) no formato do Playwright.
    async fn storage_state(&self) -> Result<Value, BrowserError> {
        Err(BrowserError::Unsupported(
            "storage_state não suportado nesta implementação".to_string(),
        ))
    }

    /// Aplica um storage state (cookies + localStorage do origin atual).
    async fn seed_storage_state(&self, state: &Value) -> Result<(), BrowserError> {
        let _ = state;
        Err(BrowserError::Unsupported(
            "seed_storage_state não suportado nesta implementação".to_string(),
        ))
    }

    /// Aplica emulação mobile (device metrics/touch/UA/locale).
    async fn set_device_profile(&self, profile: &DeviceProfile) -> Result<(), BrowserError> {
        let _ = profile;
        Err(BrowserError::Unsupported(
            "set_device_profile não suportado nesta implementação".to_string(),
        ))
    }
    /// Aguarda um seletor ficar visível.
    async fn wait_for_selector(
        &self,
        selector: &str,
        timeout: Duration,
    ) -> Result<(), BrowserError>;
    /// Texto de todos os elementos que casam com o seletor.
    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError>;
    /// Clica no primeiro elemento do seletor.
    async fn click_selector(&self, selector: &str) -> Result<(), BrowserError>;
    /// Scrolla a página.
    /// Toque real (touch) nas coordenadas do viewport (páginas com touch habilitado).
    async fn tap_at(&self, _x: f64, _y: f64) -> Result<(), BrowserError> {
        Err(BrowserError::Unsupported(
            "tap_at não suportado neste driver".to_string(),
        ))
    }

    /// Clique real (input do mouse) nas coordenadas do viewport (clique "trusted").
    async fn click_at(&self, x: f64, y: f64) -> Result<(), BrowserError> {
        self.click_at_with_modifiers(x, y, 0).await
    }

    /// Move o mouse para as coordenadas (`drag=true` mantém o botão pressionado).
    async fn mouse_move(&self, _x: f64, _y: f64, _drag: bool) -> Result<(), BrowserError> {
        Err(BrowserError::Unsupported(
            "mouse_move não suportado neste driver".to_string(),
        ))
    }

    /// Pressiona o botão esquerdo nas coordenadas.
    async fn mouse_down(&self, _x: f64, _y: f64) -> Result<(), BrowserError> {
        Err(BrowserError::Unsupported(
            "mouse_down não suportado neste driver".to_string(),
        ))
    }

    /// Solta o botão esquerdo nas coordenadas.
    async fn mouse_up(&self, _x: f64, _y: f64) -> Result<(), BrowserError> {
        Err(BrowserError::Unsupported(
            "mouse_up não suportado neste driver".to_string(),
        ))
    }

    /// Clique real com modificadores (Alt=1, Ctrl=2, Meta=4, Shift=8).
    async fn click_at_with_modifiers(
        &self,
        _x: f64,
        _y: f64,
        _modifiers: i64,
    ) -> Result<(), BrowserError> {
        Err(BrowserError::Unsupported(
            "click_at não suportado neste driver".to_string(),
        ))
    }
    /// Rola a página por um deslocamento (px).
    async fn scroll_by(&self, x: i64, y: i64) -> Result<(), BrowserError>;
    /// Captura screenshot.
    async fn screenshot(&self) -> Result<Vec<u8>, BrowserError>;
    /// Fecha a página.
    async fn close(&self) -> Result<(), BrowserError>;
}
