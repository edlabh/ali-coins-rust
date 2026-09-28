//! Diagnósticos de execução (equivalente a `libs/ui/diagnostics.js`).
//!
//! - Diretório `scratch/` criado com modo `0700`.
//! - Screenshot em falha (0600) e artefatos de DOM: hash SHA-256 do HTML
//!   normalizado e dump HTML opcional.
//! - Trace CDP entra no incremento de paridade de diagnósticos (ver
//!   `docs/05-divergencias-conhecidas.md`).

use super::driver::{BrowserError, Page};
use ali_coins_core::secure_fs::safe_write_file;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

/// Modo de captura de screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotMode {
    /// Desligado.
    Off,
    /// Sempre.
    On,
    /// Apenas em falha.
    OnlyOnFailure,
}

/// Configuração de diagnósticos.
#[derive(Debug, Clone)]
pub struct DiagnosticsOptions {
    /// Diretório de saída (`scratch/`).
    pub output_dir: PathBuf,
    /// Modo do screenshot.
    pub screenshot: ScreenshotMode,
    /// Dump do HTML bruto (`PW_DUMP_DOM`).
    pub dump_dom: bool,
}

impl Default for DiagnosticsOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("scratch"),
            screenshot: ScreenshotMode::OnlyOnFailure,
            dump_dom: false,
        }
    }
}

/// Cria o diretório de diagnósticos com `0700`.
pub fn prepare_output_dir(dir: &Path) -> Result<(), BrowserError> {
    std::fs::create_dir_all(dir).map_err(|err| BrowserError::Io(err.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Normaliza HTML para hashing (colapsa espaços em branco como o oráculo).
#[must_use]
pub fn normalize_dom(html: &str) -> String {
    html.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// SHA-256 (hex) do HTML normalizado.
#[must_use]
pub fn dom_hash(html: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_dom(html).as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Salva screenshot quando o modo/configuração pedir (0600).
pub async fn capture_screenshot(
    page: &dyn Page,
    options: &DiagnosticsOptions,
    tag: &str,
    failed: bool,
) -> Result<Option<PathBuf>, BrowserError> {
    let should_capture = match options.screenshot {
        ScreenshotMode::Off => false,
        ScreenshotMode::On => true,
        ScreenshotMode::OnlyOnFailure => failed,
    };
    if !should_capture {
        return Ok(None);
    }
    prepare_output_dir(&options.output_dir)?;
    let bytes = page.screenshot().await?;
    let path = options.output_dir.join(format!("{tag}.png"));
    safe_write_file(&path, &bytes).map_err(|err| BrowserError::Io(err.to_string()))?;
    Ok(Some(path))
}

/// Escreve o hash do DOM (`dom-<tag>.hash.txt`) e, se pedido, o HTML bruto.
pub async fn capture_dom_artifacts(
    page: &dyn Page,
    options: &DiagnosticsOptions,
    tag: &str,
) -> Result<String, BrowserError> {
    prepare_output_dir(&options.output_dir)?;
    let html = page.content().await?;
    let hash = dom_hash(&html);
    let hash_path = options.output_dir.join(format!("dom-{tag}.hash.txt"));
    safe_write_file(&hash_path, hash.as_bytes())
        .map_err(|err| BrowserError::Io(err.to_string()))?;
    if options.dump_dom {
        let dump_path = options.output_dir.join("mobile_body.html");
        safe_write_file(&dump_path, html.as_bytes())
            .map_err(|err| BrowserError::Io(err.to_string()))?;
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_e_hasheia_dom() {
        let a = dom_hash("<html>  <body>\n  <p>x</p>\n</body></html>");
        let b = dom_hash("<html> <body> <p>x</p> </body></html>");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, dom_hash("<html><body><p>y</p></body></html>"));
    }

    #[test]
    fn prepara_diretorio() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("scratch");
        prepare_output_dir(&target).expect("cria");
        assert!(target.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }
}
