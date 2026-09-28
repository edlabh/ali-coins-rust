//! Storage state (cookies + localStorage) via CDP, no formato do Playwright.
//!
//! Formato de saída (mesmas chaves do `context.storageState()`):
//! `{ cookies: [{name,value,domain,path,expires,httpOnly,secure,sameSite?}], origins: [{origin,localStorage:[{name,value}]}] }`.
//! Limitação conhecida (D-05): apenas o origin **atual** é lido/gravado para
//! localStorage (o oráculo enumera todos os origins visitados); cookies cobrem
//! todos os domínios.

use super::driver::BrowserError;
use chromiumoxide::cdp::browser_protocol::network::{
    CookieParam, CookieSameSite, GetCookiesParams, SetCookiesParams, TimeSinceEpoch,
};
use chromiumoxide::page::Page as CdpPage;
use serde_json::{Map, Value};

/// Lê o storage state da página (cookies globais + localStorage do origin atual).
pub async fn read_storage_state(page: &CdpPage) -> Result<Value, BrowserError> {
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
    let origin = origin_value
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();

    let mut origins = Vec::new();
    if origin.starts_with("http") {
        let entries_result = page
            .evaluate_expression(
                "Object.entries(localStorage).map(([name, value]) => ({ name, value }))",
            )
            .await
            .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
        let entries_value: Value = entries_result.into_value().unwrap_or(Value::Null);
        let entries = entries_value.as_array().cloned().unwrap_or_default();
        let mut origin_map = Map::new();
        origin_map.insert("origin".to_string(), Value::String(origin));
        origin_map.insert("localStorage".to_string(), Value::Array(entries));
        origins.push(Value::Object(origin_map));
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
            let Some(entries) = origin.get("localStorage").and_then(Value::as_array) else {
                continue;
            };
            if entries.is_empty() {
                continue;
            }
            // localStorage só existe em origens http(s) (about:blank é opaco):
            // falhas/restrições não devem impedir o seed dos cookies.
            let script = format!(
                "(() => {{ try {{ if (!/^https?:/.test(location.protocol)) return 0; \
                 const entries = {}; for (const e of entries) localStorage.setItem(e.name, e.value); \
                 return entries.length; }} catch (error) {{ return 0; }} }})()",
                serde_json::to_string(entries).unwrap_or_else(|_| "[]".to_string())
            );
            let _ = page.evaluate_expression(script).await;
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
