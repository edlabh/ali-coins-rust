//! Trace CDP (`Tracing.start`/`Tracing.end`) — paridade de comportamento com o
//! tracing de contexto do oráculo (`libs/ui/diagnostics.js`).
//!
//! O oráculo usa `context.tracing` do Playwright e grava um zip próprio. Aqui o
//! formato é o trace nativo do CDP em JSON (`.json`), com as mesmas regras de
//! habilitação/retensão: `PW_TRACE` (`off` em host de baixa memória,
//! `retain-on-failure` no host normal), arquivos `0600` em `PW_OUTPUT_DIR`
//! (padrão `scratch/`). Divergência registrada em
//! `docs/05-divergencias-conhecidas.md` (D-03).

use super::driver::BrowserError;
use ali_coins_core::config::EnvSource;
use ali_coins_core::secure_fs::safe_write_file;
use chromiumoxide::cdp::browser_protocol::tracing::{
    EndParams, EventDataCollected, StartParams, StartTransferMode, TraceConfig,
};
use chromiumoxide::page::Page as CdpPage;
use futures::StreamExt as _;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Categorias do trace (screenshots de timeline habilitados, como o
/// `snapshots/screenshots` do Playwright em versão reduzida).
pub const TRACE_CATEGORIES: [&str; 6] = [
    "devtools.timeline",
    "devtools.timeline.frame",
    "v8",
    "blink.user_timing",
    "disabled-by-default-devtools.screenshot",
    "disabled-by-default-devtools.timeline",
];

/// Modo do `PW_TRACE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceMode {
    /// Não inicia trace.
    Off,
    /// Inicia e mantém sempre.
    On,
    /// Inicia e mantém apenas em falha (default no host normal).
    RetainOnFailure,
    /// Inicia mas nunca mantém (valor desconhecido de `PW_TRACE`, como no oráculo).
    StartOnly,
}

/// Decide se inicia e se mantém o trace.
#[must_use]
pub fn decide_trace(mode: TraceMode, failed: bool) -> (bool, bool) {
    match mode {
        TraceMode::Off => (false, false),
        TraceMode::On => (true, true),
        TraceMode::RetainOnFailure => (true, failed),
        TraceMode::StartOnly => (true, false),
    }
}

/// Host de baixa memória (`CHROMIUM_LOW_MEMORY`, padrão ligado).
#[must_use]
pub fn is_low_memory_host(env: &EnvSource) -> bool {
    super::launch::low_memory_mode_enabled(env)
}

/// Resolve `PW_TRACE`: env explícito; sem env, `off` em low-memory e
/// `retain-on-failure` no host normal (igual a `resolveDiagnosticOption`).
#[must_use]
pub fn resolve_trace_mode(env: &EnvSource) -> TraceMode {
    let default = if is_low_memory_host(env) {
        TraceMode::Off
    } else {
        TraceMode::RetainOnFailure
    };
    let Some(raw) = env.get("PW_TRACE") else {
        return default;
    };
    let clean = raw.trim().to_ascii_lowercase();
    if clean.is_empty() {
        return default;
    }
    match clean.as_str() {
        "off" | "0" | "false" | "no" => TraceMode::Off,
        "on" | "1" | "true" => TraceMode::On,
        "retain-on-failure" => TraceMode::RetainOnFailure,
        _ => TraceMode::StartOnly,
    }
}

/// Diretório de artefatos: `PW_OUTPUT_DIR` (se não vazio) ou `scratch`.
#[must_use]
pub fn diagnostics_dir(env: &EnvSource) -> PathBuf {
    env.get("PW_OUTPUT_DIR")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("scratch"), PathBuf::from)
}

/// Nome do arquivo de trace (`<nome>-trace-<epoch_ms>.json`).
#[must_use]
pub fn trace_file_name(name: &str, timestamp_ms: i64) -> String {
    format!("{name}-trace-{timestamp_ms}.json")
}

/// Escreve o arquivo de trace com `0600` a partir dos eventos coletados.
pub fn write_trace_events(
    output_dir: &Path,
    name: &str,
    timestamp_ms: i64,
    events: &[Value],
) -> Result<PathBuf, BrowserError> {
    super::diagnostics::prepare_output_dir(output_dir)?;
    let path = output_dir.join(trace_file_name(name, timestamp_ms));
    let body = serde_json::to_vec(events).map_err(|err| BrowserError::Io(err.to_string()))?;
    safe_write_file(&path, &body).map_err(|err| BrowserError::Io(err.to_string()))?;
    Ok(path)
}

/// Trace em andamento (buffer de eventos `Tracing.dataCollected`).
pub struct Trace {
    buffer: Arc<Mutex<Vec<Value>>>,
}

impl Trace {
    /// Inicia o trace na página (best-effort: erro devolve `None`? não —
    /// devolve erro para o chamador decidir; o oráculo ignora falhas).
    pub async fn start(page: &CdpPage) -> Result<Self, BrowserError> {
        let mut events = page
            .event_listener::<EventDataCollected>()
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        let buffer: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = Arc::clone(&buffer);
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let Ok(mut guard) = sink.lock() {
                    guard.extend(event.value.iter().cloned());
                }
            }
        });
        let trace_config = TraceConfig {
            included_categories: Some(
                TRACE_CATEGORIES
                    .iter()
                    .map(|item| (*item).to_string())
                    .collect(),
            ),
            excluded_categories: Some(vec!["*".to_string()]),
            ..TraceConfig::default()
        };
        page.execute(StartParams {
            buffer_usage_reporting_interval: None,
            transfer_mode: Some(StartTransferMode::ReportEvents),
            stream_format: None,
            stream_compression: None,
            trace_config: Some(trace_config),
            perfetto_config: None,
            tracing_backend: None,
        })
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;
        Ok(Self { buffer })
    }

    /// Eventos coletados até agora (cópia).
    #[must_use]
    pub fn events(&self) -> Vec<Value> {
        self.buffer
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Finaliza o trace e grava o arquivo quando `keep`.
    pub async fn stop(
        self,
        page: &CdpPage,
        output_dir: &Path,
        name: &str,
        keep: bool,
    ) -> Result<Option<PathBuf>, BrowserError> {
        page.execute(EndParams {})
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        // Dá um tempo curto para os últimos `dataCollected` chegarem.
        tokio::time::sleep(Duration::from_millis(300)).await;
        if !keep {
            return Ok(None);
        }
        let events = self.events();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
            });
        write_trace_events(output_dir, name, timestamp, &events).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn modo_default_respeita_low_memory() {
        assert_eq!(resolve_trace_mode(&env(&[])), TraceMode::Off);
        assert_eq!(
            resolve_trace_mode(&env(&[("CHROMIUM_LOW_MEMORY", "0")])),
            TraceMode::RetainOnFailure
        );
    }

    #[test]
    fn modo_explicito_tem_precedencia() {
        assert_eq!(
            resolve_trace_mode(&env(&[("CHROMIUM_LOW_MEMORY", "0"), ("PW_TRACE", "off")])),
            TraceMode::Off
        );
        assert_eq!(
            resolve_trace_mode(&env(&[("PW_TRACE", "on")])),
            TraceMode::On
        );
        assert_eq!(
            resolve_trace_mode(&env(&[("PW_TRACE", " retain-on-failure ")])),
            TraceMode::RetainOnFailure
        );
        // Valor desconhecido inicia mas nunca mantém (igual ao oráculo).
        assert_eq!(
            resolve_trace_mode(&env(&[("PW_TRACE", "qualquer")])),
            TraceMode::StartOnly
        );
        // Vazio cai no default do host.
        assert_eq!(
            resolve_trace_mode(&env(&[("PW_TRACE", "  ")])),
            TraceMode::Off
        );
    }

    #[test]
    fn decisao_de_inicio_e_retencao() {
        assert_eq!(decide_trace(TraceMode::Off, true), (false, false));
        assert_eq!(decide_trace(TraceMode::On, false), (true, true));
        assert_eq!(
            decide_trace(TraceMode::RetainOnFailure, false),
            (true, false)
        );
        assert_eq!(decide_trace(TraceMode::RetainOnFailure, true), (true, true));
        assert_eq!(decide_trace(TraceMode::StartOnly, true), (true, false));
    }

    #[test]
    fn diretorio_e_nome_do_trace() {
        assert_eq!(diagnostics_dir(&env(&[])), PathBuf::from("scratch"));
        assert_eq!(
            diagnostics_dir(&env(&[("PW_OUTPUT_DIR", "/tmp/artefatos")])),
            PathBuf::from("/tmp/artefatos")
        );
        assert_eq!(
            diagnostics_dir(&env(&[("PW_OUTPUT_DIR", "   ")])),
            PathBuf::from("scratch")
        );
        assert_eq!(
            trace_file_name("mobile", 1_756_000_000_000),
            "mobile-trace-1756000000000.json"
        );
    }

    #[test]
    fn grava_eventos_com_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let events = vec![serde_json::json!({"name": "navigationStart"})];
        let path = write_trace_events(dir.path(), "ctx", 42, &events).expect("grava");
        let body = std::fs::read_to_string(&path).expect("lê");
        assert_eq!(body, r#"[{"name":"navigationStart"}]"#);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
