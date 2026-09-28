//! Filtro de storage (equivalente a `libs/storage_filter.js`).

use serde_json::{Map, Value};

const ALLOWED_SUBSTRINGS: [&str; 10] = [
    "login", "user", "account", "token", "session", "auth", "_m_h5_tk", "currency", "locale",
    "lang",
];

/// Verifica se uma chave de localStorage está na allowlist (substring, case-insensitive).
#[must_use]
pub fn is_allowed_storage_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let lower = key.to_lowercase();
    ALLOWED_SUBSTRINGS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Truthiness do JavaScript para os tipos JSON.
fn is_js_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(_) | Value::Object(_) => false,
    }
}

/// Remove chaves fora da allowlist, preservando cookies e o restante do estado.
///
/// Retorna o estado original quando `origins` não é um array (igual ao oráculo).
#[must_use]
pub fn filter_storage_state(state: Value) -> Value {
    let Value::Object(mut root) = state else {
        return state;
    };
    let Some(Value::Array(origins)) = root.get("origins").cloned() else {
        return Value::Object(root);
    };

    let mapped: Vec<Value> = origins
        .into_iter()
        .map(|origin| {
            if is_js_falsy(&origin) {
                return origin;
            }
            let origin_obj = match origin {
                Value::Object(obj) => obj,
                Value::Array(items) => {
                    let mut obj = Map::new();
                    for (index, item) in items.into_iter().enumerate() {
                        obj.insert(index.to_string(), item);
                    }
                    obj
                }
                Value::String(text) => {
                    let mut obj = Map::new();
                    for (index, ch) in text.chars().enumerate() {
                        obj.insert(index.to_string(), Value::String(ch.to_string()));
                    }
                    obj
                }
                // Números/true: o spread não copia propriedades próprias enumeráveis.
                _ => Map::new(),
            };
            let mut origin_obj = origin_obj;
            match origin_obj.get("localStorage").cloned() {
                Some(Value::Array(entries)) => {
                    let filtered: Vec<Value> = entries
                        .into_iter()
                        .filter(|entry| {
                            entry
                                .get("name")
                                .and_then(Value::as_str)
                                .is_some_and(is_allowed_storage_key)
                        })
                        .collect();
                    origin_obj.insert("localStorage".to_string(), Value::Array(filtered));
                }
                _ => {
                    origin_obj.insert("localStorage".to_string(), Value::Array(Vec::new()));
                }
            }
            Value::Object(origin_obj)
        })
        .collect();

    root.insert("origins".to_string(), Value::Array(mapped));
    Value::Object(root)
}

/// Decide se a filtragem estrita deve ser aplicada (default: ligada).
#[must_use]
pub fn should_filter_storage(
    options: &super::SessionOptions,
    env: &crate::config::EnvSource,
) -> bool {
    if let Some(explicit) = options.filter_storage {
        return explicit;
    }
    match env.get("SESSION_STRICT_STORAGE") {
        None | Some("") => true,
        Some(value) => !["false", "0", "off", "no"].contains(&value.to_lowercase().as_str()),
    }
}

/// Constrói um storage state de teste com cookies e localStorage.
#[must_use]
pub fn storage_state_json(cookies: &[(&str, &str)], origins: &[(&str, &[(&str, &str)])]) -> Value {
    let cookies: Vec<Value> = cookies
        .iter()
        .map(|(name, value)| {
            let mut cookie = Map::new();
            cookie.insert("name".to_string(), Value::String((*name).to_string()));
            cookie.insert("value".to_string(), Value::String((*value).to_string()));
            Value::Object(cookie)
        })
        .collect();
    let origins: Vec<Value> = origins
        .iter()
        .map(|(origin, entries)| {
            let entries: Vec<Value> = entries
                .iter()
                .map(|(name, value)| {
                    let mut item = Map::new();
                    item.insert("name".to_string(), Value::String((*name).to_string()));
                    item.insert("value".to_string(), Value::String((*value).to_string()));
                    Value::Object(item)
                })
                .collect();
            let mut obj = Map::new();
            obj.insert("origin".to_string(), Value::String((*origin).to_string()));
            obj.insert("localStorage".to_string(), Value::Array(entries));
            Value::Object(obj)
        })
        .collect();
    let mut state = Map::new();
    state.insert("cookies".to_string(), Value::Array(cookies));
    state.insert("origins".to_string(), Value::Array(origins));
    Value::Object(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_por_substring() {
        assert!(is_allowed_storage_key("userInfo"));
        assert!(is_allowed_storage_key("AUTH_TOKEN"));
        assert!(is_allowed_storage_key("_m_h5_tk"));
        assert!(is_allowed_storage_key("aplus_currency"));
        assert!(!is_allowed_storage_key("APLUS_S_CORE"));
        assert!(!is_allowed_storage_key("aegis"));
        assert!(!is_allowed_storage_key(""));
    }

    #[test]
    fn filtra_localstorage_e_preserva_cookies() {
        let state = storage_state_json(
            &[("xman_us_t", "abc")],
            &[(
                "https://www.aliexpress.com",
                &[("user", "1"), ("aegis", "ruido"), ("token", "t")],
            )],
        );
        let filtered = filter_storage_state(state);
        let origins = filtered["origins"].as_array().unwrap();
        let entries = origins[0]["localStorage"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(filtered["cookies"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn sem_origins_retorna_original() {
        let state = serde_json::json!({ "cookies": [] });
        assert_eq!(filter_storage_state(state.clone()), state);
    }

    #[test]
    fn localstorage_nao_array_vira_vazio() {
        let state = serde_json::json!({
            "cookies": [],
            "origins": [{ "origin": "https://x", "localStorage": "invalido" }]
        });
        let filtered = filter_storage_state(state);
        assert_eq!(
            filtered["origins"][0]["localStorage"],
            serde_json::json!([])
        );
    }
}
