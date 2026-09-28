//! Camada HTTP segura para notificações (equivalente a `safeFetch` do oráculo).
//!
//! - Pré-checagem por hop com [`validate_external_url`] (loopback/privados,
//!   protocolo, hostnames bloqueados).
//! - **Pinning de DNS**: um resolver customizado resolve uma vez, descarta IPs
//!   privados e devolve apenas endereços validados para a conexão (anti
//!   DNS rebinding) — cada conexão passa pela validação.
//! - Redirects manuais (máx. 5), revalidados a cada hop; `303` e `301/302` de
//!   POST viram GET sem corpo; cabeçalhos sensíveis são descartados ao trocar
//!   de origem.
//! - Erros de bloqueio carregam o marcador `SSRF_BLOCKED` (contrato do oráculo).

use crate::url_guard::{is_private_ip, validate_external_url};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Máximo de redirects seguidos.
pub const MAX_REDIRECTS: usize = 5;
const SSRF_MARKER: &str = "SSRF_BLOCKED";

/// Erros da camada HTTP segura.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// Destino bloqueado por SSRF (marcador `SSRF_BLOCKED`).
    #[error("SSRF_BLOCKED: {0}")]
    SsrfBlocked(String),
    /// Excesso de redirects.
    #[error("Máximo de redirects excedido ({0}).")]
    TooManyRedirects(usize),
    /// Falha de rede/HTTP.
    #[error("{0}")]
    Request(String),
    /// Timeout ambíguo (nunca retentado em notificações).
    #[error("timeout: {0}")]
    Timeout(String),
    /// URL inicial malformada.
    #[error("URL malformada: {0}")]
    InvalidUrl(String),
}

impl HttpError {
    /// É um bloqueio SSRF?
    #[must_use]
    pub fn is_ssrf_blocked(&self) -> bool {
        matches!(self, Self::SsrfBlocked(_))
    }

    /// É um timeout ambíguo?
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }
}

/// Resposta de uma requisição segura.
#[derive(Debug, Clone)]
pub struct SafeResponse {
    /// Status HTTP final.
    pub status: u16,
    /// Corpo em texto (UTF-8 lossy).
    pub body: String,
    /// URL final após redirects.
    pub final_url: String,
}

/// Resolver que valida IPs no momento da conexão (pinning).
#[derive(Debug, Clone, Copy)]
pub struct PinningResolver;

impl Resolve for PinningResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let safe: Vec<SocketAddr> = addrs
                .filter(|addr| !is_private_ip(&addr.ip().to_string()))
                .collect();
            if safe.is_empty() {
                return Err(Box::new(std::io::Error::other(format!(
                    "{SSRF_MARKER}: host resolve apenas para IPs privados"
                )))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(safe.into_iter()) as Addrs)
        })
    }
}

/// Cliente HTTP com validação SSRF por hop.
pub struct SafeHttpClient {
    pinned: reqwest::Client,
    permissive: reqwest::Client,
    allow_private: bool,
    max_redirects: usize,
}

impl std::fmt::Debug for SafeHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SafeHttpClient")
            .field("allow_private", &self.allow_private)
            .field("max_redirects", &self.max_redirects)
            .finish_non_exhaustive()
    }
}

impl SafeHttpClient {
    /// Cria o cliente (timeout por requisição e opt-in de destinos privados).
    pub fn new(allow_private: bool, timeout: Duration) -> Result<Self, HttpError> {
        let pinned = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PinningResolver))
            .build()
            .map_err(|err| HttpError::Request(err.to_string()))?;
        let permissive = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| HttpError::Request(err.to_string()))?;
        Ok(Self {
            pinned,
            permissive,
            allow_private,
            max_redirects: MAX_REDIRECTS,
        })
    }

    /// GET simples.
    pub async fn get(&self, url: &str) -> Result<SafeResponse, HttpError> {
        self.execute(reqwest::Method::GET, url, None, None).await
    }

    /// POST de texto com content-type.
    pub async fn post_text(
        &self,
        url: &str,
        body: &str,
        content_type: &str,
    ) -> Result<SafeResponse, HttpError> {
        self.execute(
            reqwest::Method::POST,
            url,
            Some(body.as_bytes().to_vec()),
            Some(content_type.to_string()),
        )
        .await
    }

    /// POST de JSON.
    pub async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<SafeResponse, HttpError> {
        let payload =
            serde_json::to_vec(body).map_err(|err| HttpError::Request(err.to_string()))?;
        self.execute(
            reqwest::Method::POST,
            url,
            Some(payload),
            Some("application/json".to_string()),
        )
        .await
    }

    async fn execute(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
        content_type: Option<String>,
    ) -> Result<SafeResponse, HttpError> {
        let mut current_url = url.to_string();
        let mut current_method = method;
        let mut current_body = body;
        let mut current_content_type = content_type;
        let mut redirects = 0_usize;

        loop {
            let validation = validate_external_url(
                Some(&current_url),
                Some(self.allow_private),
                false,
                &crate::config::EnvSource::default(),
            );
            if !validation.ok {
                return Err(HttpError::SsrfBlocked(
                    validation
                        .reason
                        .unwrap_or_else(|| "destino bloqueado".to_string()),
                ));
            }

            let client = if self.allow_private {
                &self.permissive
            } else {
                &self.pinned
            };
            let mut request = client.request(current_method.clone(), &current_url);
            if let Some(content_type) = &current_content_type {
                request = request.header(reqwest::header::CONTENT_TYPE, content_type);
            }
            if let Some(body) = &current_body {
                request = request.body(body.clone());
            }

            let response = request.send().await.map_err(map_reqwest_error)?;
            let status = response.status();
            let response_url = response.url().clone();

            if status.is_redirection() {
                if redirects >= self.max_redirects {
                    return Err(HttpError::TooManyRedirects(self.max_redirects));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        HttpError::Request("redirect sem cabeçalho Location".to_string())
                    })?;
                let next = response_url
                    .join(&location)
                    .map_err(|err| HttpError::InvalidUrl(format!("{location} ({err})")))?;
                redirects += 1;

                // 303 sempre vira GET; 301/302 de POST viram GET sem corpo.
                if status == reqwest::StatusCode::SEE_OTHER
                    || ((status == reqwest::StatusCode::MOVED_PERMANENTLY
                        || status == reqwest::StatusCode::FOUND)
                        && current_method == reqwest::Method::POST)
                {
                    current_method = reqwest::Method::GET;
                    current_body = None;
                    current_content_type = None;
                }

                current_url = next.to_string();
                continue;
            }

            let final_url = response_url.to_string();
            let bytes = response.bytes().await.map_err(map_reqwest_error)?;
            return Ok(SafeResponse {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
                final_url,
            });
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_reqwest_error(err: reqwest::Error) -> HttpError {
    if err.is_timeout() {
        return HttpError::Timeout(err.to_string());
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&err);
    while let Some(current) = source {
        if current.to_string().contains(SSRF_MARKER) {
            return HttpError::SsrfBlocked(current.to_string());
        }
        source = current.source();
    }
    HttpError::Request(err.to_string())
}
