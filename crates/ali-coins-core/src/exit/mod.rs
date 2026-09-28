//! Encerramento seguro e crash handler (equivalente a `libs/exit.js` + `libs/crash.js`).
//!
//! - Códigos de saída padronizados 0–6 e 130/143 para sinais.
//! - Flush explícito de stdout/stderr/logger antes de `process::exit`.
//! - Panic hook global: loga `fatal`, roda cleanup best-effort (teto de 15 s) e
//!   encerra com exit code 6; segundo panic durante o encerramento também sai 6.

use crate::config::LogLevel;
use crate::logging;
use serde_json::Value;
use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Teto de espera pelo flush dos streams (igual ao oráculo).
pub const FLUSH_TIMEOUT_MS: u64 = 5000;
/// Teto de espera pelo cleanup no crash (igual ao oráculo).
pub const CRASH_TIMEOUT_MS: u64 = 15_000;

/// Códigos de saída padronizados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// Sucesso.
    Success = 0,
    /// Falha crítica/credenciais.
    Failure = 1,
    /// Sem ação / já coletado.
    NoAction = 2,
    /// Lock ativo.
    LockActive = 3,
    /// Streak quebrado.
    StreakBroken = 4,
    /// 2FA não-interativo.
    TwoFactor = 5,
    /// Falha global (crash).
    Crash = 6,
}

impl ExitCode {
    /// Código numérico.
    #[must_use]
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Código convencional para um sinal (128 + número).
#[must_use]
pub fn signal_exit_code(signal: &str) -> i32 {
    match signal {
        "SIGINT" => 130,
        "SIGTERM" => 143,
        _ => 1,
    }
}

/// Flush de stdout/stderr/logger e encerramento imediato.
pub fn flush_and_exit(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    logging::global().flush();
    std::process::exit(code);
}

/// Instala o panic hook global com cleanup best-effort e exit code 6.
///
/// O cleanup roda em thread separada com teto de [`CRASH_TIMEOUT_MS`]; um segundo
/// panic durante o encerramento sai imediatamente com código 6.
pub fn install_panic_handler(cleanup: impl Fn() + Send + Sync + 'static) {
    let cleanup = Arc::new(cleanup);
    let is_exiting = Arc::new(AtomicBool::new(false));

    std::panic::set_hook(Box::new(move |panic_info| {
        if is_exiting.swap(true, Ordering::SeqCst) {
            flush_and_exit(ExitCode::Crash.as_i32());
        }

        let payload = if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "panic sem payload textual".to_string()
        };
        let location = panic_info.location().map_or_else(
            || "desconhecido".to_string(),
            |location| format!("{}:{}", location.file(), location.line()),
        );
        logging::global().fatal(
            "Falha global não tratada detectada (panic). Finalizando execução com código 6.",
            &[
                ("err", Value::String(payload)),
                ("crashType", Value::String("panic".to_string())),
                ("location", Value::String(location)),
            ],
        );

        let (sender, receiver) = std::sync::mpsc::channel();
        let cleanup = Arc::clone(&cleanup);
        std::thread::spawn(move || {
            cleanup();
            let _ = sender.send(());
        });
        let _ = receiver.recv_timeout(Duration::from_millis(CRASH_TIMEOUT_MS));

        flush_and_exit(ExitCode::Crash.as_i32());
    }));
}

/// Loga a mensagem fatal e encerra com o código informado (uso em `main`).
pub fn fatal_exit(message: &str, code: ExitCode) -> ! {
    logging::global().log(
        LogLevel::Fatal,
        message,
        &[("crashType", Value::String("explicit".to_string()))],
    );
    flush_and_exit(code.as_i32());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codigos_de_saida() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::Failure.as_i32(), 1);
        assert_eq!(ExitCode::NoAction.as_i32(), 2);
        assert_eq!(ExitCode::LockActive.as_i32(), 3);
        assert_eq!(ExitCode::StreakBroken.as_i32(), 4);
        assert_eq!(ExitCode::TwoFactor.as_i32(), 5);
        assert_eq!(ExitCode::Crash.as_i32(), 6);
    }

    #[test]
    fn codigos_de_sinal() {
        assert_eq!(signal_exit_code("SIGINT"), 130);
        assert_eq!(signal_exit_code("SIGTERM"), 143);
        assert_eq!(signal_exit_code("SIGHUP"), 1);
    }
}
