//! Storage state (cookies + localStorage) via CDP, no formato do Playwright.
//!
//! Formato de saída (mesmas chaves do `context.storageState()`):
//! `{ cookies: [{name,value,domain,path,expires,httpOnly,secure,sameSite?}], origins: [{origin,localStorage:[{name,value}]}] }`.
//! D-05 concluído: o localStorage é lido/gravado por origin via CDP
//! (`DOMStorage`), incluindo **todos os origins visitados** — como o
//! `storageState()` do Playwright.

use super::driver::BrowserError;
use chromiumoxide::cdp::browser_protocol::dom_storage::{
    GetDomStorageItemsParams, SetDomStorageItemParams, StorageId,
};
use chromiumoxide::cdp::browser_protocol::network::{
    CookieParam, CookieSameSite, GetCookiesParams, SetCookiesParams, TimeSinceEpoch,
};
use chromiumoxide::page::Page as CdpPage;
use serde_json::{Map, Value};

/// Origins para o storage state: visitados (http) + o atual, sem repetições.
fn merge_origins(visited: &[String], current: Option<&str>) -> Vec<String> {
    let mut origins: Vec<String> = Vec::new();
    for origin in visited.iter().map(String::as_str).chain(current) {
        if origin.starts_with("http") && !origins.iter().any(|item| item == origin) {
            origins.push(origin.to_string());
        }
    }
    origins
}

/// Storage ID de localStorage para um origin.
fn local_storage_id(origin: &str) -> Result<StorageId, BrowserError> {
    StorageId::builder()
        .security_origin(origin.to_string())
        .is_local_storage(true)
        .build()
        .map_err(|err| BrowserError::Evaluate(err.clone()))
}

/// Lê o storage state (cookies globais + localStorage de todos os origins visitados).
pub async fn read_storage_state(
    page: &CdpPage,
    visited_origins: &[String],
) -> Result<Value, BrowserError> {
    let cookies_return = page
        .execute(GetCookiesParams { urls: None })
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;

    let cookies: Vec<Value> = cookies_return
        .cookies
        .iter()
        .map(|cookie| {
            let mut map = Map::new();
            map.insert("name".to_string(), Value::String(cookie.name.clone()));
            map.insert("value".to_string(), Value::String(cookie.value.clone()));
            map.insert("domain".to_string(), Value::String(cookie.domain.clone()));
            map.insert("path".to_string(), Value::String(cookie.path.clone()));
            map.insert("expires".to_string(), Value::from(cookie.expires));
            map.insert("httpOnly".to_string(), Value::Bool(cookie.http_only));
            map.insert("secure".to_string(), Value::Bool(cookie.secure));
            if let Some(same_site) = &cookie.same_site {
                map.insert(
                    "sameSite".to_string(),
                    Value::String(same_site_name(same_site).to_string()),
                );
            }
            Value::Object(map)
        })
        .collect();

    let origin_result = page
        .evaluate_expression("location.origin")
        .await
        .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
    let origin_value: Value = origin_result.into_value().unwrap_or(Value::Null);
    let current_origin = origin_value.as_str();

    let mut origins = Vec::new();
    for origin in merge_origins(visited_origins, current_origin) {
        let items: Vec<Value> = if Some(origin.as_str()) == current_origin {
            // Origin atual: leitura direta pelo JS da própria página (robusta).
            let entries_result = page
                .evaluate_expression(
                    "Object.entries(localStorage).map(([name, value]) => ({ name, value }))",
                )
                .await
                .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
            let entries_value: Value = entries_result.into_value().unwrap_or(Value::Null);
            entries_value.as_array().cloned().unwrap_or_default()
        } else {
            // Demais origins visitados: DOMStorage (best-effort — o origin de
            // outra página/aba pode não ter frame neste target CDP).
            match page
                .execute(GetDomStorageItemsParams::new(local_storage_id(&origin)?))
                .await
            {
                Ok(entries) => entries
                    .entries
                    .iter()
                    .filter_map(|item| {
                        let pair = item.inner();
                        (pair.len() >= 2)
                            .then(|| serde_json::json!({ "name": pair[0], "value": pair[1] }))
                    })
                    .collect(),
                Err(_) => continue,
            }
        };
        origins.push(serde_json::json!({
            "origin": origin,
            "localStorage": items,
        }));
    }

    let mut state = Map::new();
    state.insert("cookies".to_string(), Value::Array(cookies));
    state.insert("origins".to_string(), Value::Array(origins));
    Ok(Value::Object(state))
}

/// Aplica cookies (todos os domínios) e localStorage do origin atual.
pub async fn seed_storage_state(page: &CdpPage, state: &Value) -> Result<(), BrowserError> {
    let mut cookies = Vec::new();
    if let Some(items) = state.get("cookies").and_then(Value::as_array) {
        for item in items {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            let value = item
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut builder = CookieParam::builder().name(name).value(value);
            if let Some(domain) = item.get("domain").and_then(Value::as_str) {
                builder = builder.domain(domain);
            }
            if let Some(path) = item.get("path").and_then(Value::as_str) {
                builder = builder.path(path);
            }
            if let Some(secure) = item.get("secure").and_then(Value::as_bool) {
                builder = builder.secure(secure);
            }
            if let Some(http_only) = item.get("httpOnly").and_then(Value::as_bool) {
                builder = builder.http_only(http_only);
            }
            if let Some(expires) = item.get("expires").and_then(Value::as_f64) {
                if expires > 0.0 {
                    builder = builder.expires(TimeSinceEpoch::new(expires));
                }
            }
            if let Some(same_site) = item.get("sameSite").and_then(Value::as_str) {
                if let Some(parsed) = parse_same_site(same_site) {
                    builder = builder.same_site(parsed);
                }
            }
            match builder.build() {
                Ok(cookie) => cookies.push(cookie),
                Err(err) => return Err(BrowserError::Launch(err.clone())),
            }
        }
    }
    if !cookies.is_empty() {
        page.execute(SetCookiesParams { cookies })
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
    }

    if let Some(origins) = state.get("origins").and_then(Value::as_array) {
        for origin in origins {
            let Some(origin_value) = origin.get("origin").and_then(Value::as_str) else {
                continue;
            };
            if !origin_value.starts_with("http") {
                continue;
            }
            let Some(entries) = origin.get("localStorage").and_then(Value::as_array) else {
                continue;
            };
            let storage_id = local_storage_id(origin_value)?;
            for entry in entries {
                let (Some(name), Some(value)) = (
                    entry.get("name").and_then(Value::as_str),
                    entry.get("value").and_then(Value::as_str),
                ) else {
                    continue;
                };
                // Best-effort por item: um par inválido não derruba o seed dos demais.
                let _ = page
                    .execute(SetDomStorageItemParams::new(
                        storage_id.clone(),
                        name.to_string(),
                        value.to_string(),
                    ))
                    .await;
            }
        }
    }
    Ok(())
}

fn same_site_name(same_site: &CookieSameSite) -> &'static str {
    match same_site {
        CookieSameSite::Strict => "Strict",
        CookieSameSite::Lax => "Lax",
        CookieSameSite::None => "None",
    }
}

fn parse_same_site(raw: &str) -> Option<CookieSameSite> {
    match raw.to_ascii_lowercase().as_str() {
        "strict" => Some(CookieSameSite::Strict),
        "lax" => Some(CookieSameSite::Lax),
        "none" => Some(CookieSameSite::None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junta_origins_visitados_sem_repetir() {
        let visited = vec!["https://m.aliexpress.com".to_string()];
        let merged = merge_origins(&visited, Some("https://www.aliexpress.com"));
        assert_eq!(
            merged,
            vec![
                "https://m.aliexpress.com".to_string(),
                "https://www.aliexpress.com".to_string()
            ]
        );
        // Repetido e about:blank são ignorados.
        let merged = merge_origins(&visited, Some("https://m.aliexpress.com"));
        assert_eq!(merged, visited);
        let merged = merge_origins(&[], Some("about:blank"));
        assert!(merged.is_empty());
    }

    #[test]
    fn same_site_ida_e_volta() {
        for (raw, expected) in [
            ("Strict", CookieSameSite::Strict),
            ("lax", CookieSameSite::Lax),
            ("NONE", CookieSameSite::None),
        ] {
            let parsed = parse_same_site(raw).expect("válido");
            assert_eq!(parsed, expected);
            assert_eq!(
                same_site_name(&parsed),
                match expected {
                    CookieSameSite::Strict => "Strict",
                    CookieSameSite::Lax => "Lax",
                    CookieSameSite::None => "None",
                }
            );
        }
        assert!(parse_same_site("invalido").is_none());
    }
}
