//! Binário `ali-coins` (Rust).
//!
//! Fase 0: apenas esqueleto. Os subcomandos (`all`, `checkin`, `tasks`,
//! `export-session`, `import-session`) e as flags compatíveis com o contrato
//! C-02 serão implementados a partir da fase 1 (ver `docs/03-roadmap.md`).

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    eprintln!(
        "ali-coins-rust {} — Fase 0: esqueleto do workspace, nenhum subcomando implementado.",
        env!("CARGO_PKG_VERSION")
    );
    std::process::ExitCode::from(1)
}
