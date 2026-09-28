//! Bloqueio de recursos via `Fetch` (equivalente ao `context.route` do oráculo).
//!
//! Aborta imagens/mídia/fontes e hosts de telemetria; todo o resto segue
//! (`continue`). A decisão pura vive em `launch::should_block_resource`; aqui
//! ficam os patterns do CDP e o loop de eventos `Fetch.requestPaused`.

use super::driver::BrowserError;
use super::launch::should_block_resource;
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams, EventRequestPaused, FailRequestParams, RequestPattern,
};
use chromiumoxide::cdp::browser_protocol::network::{ErrorReason, ResourceType};
use chromiumoxide::page::Page as CdpPage;
use futures::StreamExt as _;

/// Hosts de telemetria (espelho de `launch::TELEMETRY_HOSTS` para patterns).
pub const TELEMETRY_PATTERNS: [&str; 8] = [
    "*umeng*",
    "*google-analytics*",
    "*/googletagmanager/*",
    "*doubleclick*",
    "*facebook.net*",
    "*tiktok*",
    "*criteo*",
    "*bing*",
];

/// Patterns do `Fetch.enable` para recursos que queremos interceptar.
#[must_use]
pub fn blocking_patterns(allow_media: bool) -> Vec<RequestPattern> {
    let mut patterns = Vec::new();
    if !allow_media {
        for resource_type in [ResourceType::Image, ResourceType::Media, ResourceType::Font] {
            patterns.push(RequestPattern {
                url_pattern: None,
                resource_type: Some(resource_type),
                request_stage: None,
            });
        }
    }
    for url_pattern in TELEMETRY_PATTERNS {
        patterns.push(RequestPattern {
            url_pattern: Some(url_pattern.to_string()),
            resource_type: None,
            request_stage: None,
        });
    }
    patterns
}

/// Nome do `ResourceType` no vocabulário do `launch::should_block_resource`.
fn resource_name(resource_type: &ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Image => "image",
        ResourceType::Media => "media",
        ResourceType::Font => "font",
        _ => "other",
    }
}

/// Habilita o bloqueio na página e mantém o loop de decisão em background.
pub async fn enable_resource_blocking(
    page: &CdpPage,
    allow_media: bool,
) -> Result<(), BrowserError> {
    let patterns = blocking_patterns(allow_media);
    if patterns.is_empty() {
        return Ok(());
    }
    page.execute(EnableParams {
        patterns: Some(patterns),
        handle_auth_requests: Some(false),
    })
    .await
    .map_err(|err| BrowserError::Launch(err.to_string()))?;

    let mut events = page
        .event_listener::<EventRequestPaused>()
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;
    let page = page.clone();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            let block = should_block_resource(
                resource_name(&event.resource_type),
                &event.request.url,
                allow_media,
            );
            let request_id = event.request_id.clone();
            let failed = if block {
                page.execute(FailRequestParams::new(request_id, ErrorReason::Aborted))
                    .await
                    .is_err()
            } else {
                page.execute(ContinueRequestParams::new(request_id))
                    .await
                    .is_err()
            };
            if failed {
                break;
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_de_bloqueio() {
        let patterns = blocking_patterns(false);
        assert_eq!(patterns.len(), 3 + TELEMETRY_PATTERNS.len());
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.resource_type == Some(ResourceType::Image))
        );

        let patterns = blocking_patterns(true);
        assert_eq!(patterns.len(), TELEMETRY_PATTERNS.len());
        assert!(
            patterns
                .iter()
                .all(|pattern| pattern.resource_type.is_none())
        );
    }
}
