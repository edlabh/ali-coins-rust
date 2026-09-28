//! Camada de browser do `ali-coins-rust`.
//!
//! Conterá o trait `BrowserDriver`/`Page` e as implementações `CdpDriver`
//! (chromiumoxide, ADR-0001), `MockDriver` (testes) e `SidecarPlaywrightDriver`
//! (oráculo, atrás de feature flag). Nada será implementado antes da fase 2.

#![forbid(unsafe_code)]

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
