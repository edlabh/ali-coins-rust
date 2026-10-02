//! Lockfile exclusivo compatível com `lockfile.js` do oráculo.
//!
//! Publicação atômica por hardlink de um temporário completo, classificação de
//! lock stale/órfão/symlink, remoção condicional anti-TOCTOU, refresh periódico
//! por mtime (fallback: reescrita condicional) e erro tipado `LOCK_ACTIVE`
//! (exit code 3 no CLI).

use crate::secure_fs::safe_chmod_600;
use chrono::{DateTime, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Stale timeout padrão (30 min).
pub const DEFAULT_STALE_TIMEOUT_MS: u64 = 30 * 60 * 1000;
/// Tentativas de aquisição após limpeza de locks órfãos/stale/symlink.
pub const MAX_ACQUIRE_ATTEMPTS: u32 = 5;
const LOCK_READ_GRACE_MS: u64 = 900;
const LOCK_READ_RETRY_MS: u64 = 150;
const CLOCK_SKEW_TOLERANCE_MS: i64 = 5 * 60 * 1000;
/// Código de "já existe" do sistema (`EEXIST` no Unix; `ERROR_ALREADY_EXISTS` no Windows).
#[cfg(unix)]
const ERR_EXISTS: i32 = 17;
#[cfg(windows)]
const ERR_EXISTS: i32 = 183;
#[cfg(not(any(unix, windows)))]
const ERR_EXISTS: i32 = 17;

/// Códigos de erro de FS sem suporte a hardlink (por SO).
#[cfg(unix)]
const LINK_UNSUPPORTED_CODES: [i32; 5] = [
    18, // EXDEV
    1,  // EPERM
    38, // ENOSYS
    95, // EOPNOTSUPP / ENOTSUP
    31, // EMLINK
];
#[cfg(windows)]
const LINK_UNSUPPORTED_CODES: [i32; 5] = [
    1,    // ERROR_INVALID_FUNCTION
    17,   // ERROR_NOT_SAME_DEVICE
    50,   // ERROR_NOT_SUPPORTED
    5,    // ERROR_ACCESS_DENIED (cai para O_EXCL)
    1314, // ERROR_PRIVILEGE_NOT_HELD
];
#[cfg(not(any(unix, windows)))]
const LINK_UNSUPPORTED_CODES: [i32; 5] = [18, 1, 38, 95, 31];

/// Conteúdo do lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockData {
    /// PID do dono.
    pub pid: u32,
    /// Identidade da geração (UUID v4).
    pub lock_id: String,
    /// Data de criação/renovação (ISO).
    pub created_at: String,
    /// Hostname do dono.
    pub host: String,
    /// Plataforma (`linux`/`darwin`/`win32`).
    pub platform: String,
}

/// Erros do lockfile.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Lock ativo de outra execução (exit 3 no CLI).
    #[error("{message}")]
    Active {
        /// Mensagem PT-BR do oráculo.
        message: String,
        /// Falha transitória de I/O (tratada como ativo por segurança).
        io_error: bool,
    },
    /// Erro de I/O não recuperável.
    #[error("{0}")]
    Io(String),
    /// Falha genérica (ex.: 5 tentativas esgotadas).
    #[error("{0}")]
    Other(String),
}

impl LockError {
    /// É um lock ativo (`LOCK_ACTIVE`)?
    #[must_use]
    pub fn is_lock_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    /// Foi classificado como erro de I/O transitório?
    #[must_use]
    pub fn is_io_error(&self) -> bool {
        matches!(self, Self::Active { io_error: true, .. })
    }
}

/// Opções de aquisição.
#[derive(Debug, Clone)]
pub struct LockOptions {
    /// Caminho do lockfile.
    pub path: PathBuf,
    /// `--force`: remove lock existente mesmo ativo.
    pub force: bool,
    /// Stale timeout (default: 30 min).
    pub stale_timeout_ms: Option<u64>,
    /// Intervalo de refresh (default: stale/3, teto 5 min, piso 50 ms).
    pub refresh_interval_ms: Option<u64>,
}

impl LockOptions {
    /// Opções para um caminho com defaults do oráculo.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            force: false,
            stale_timeout_ms: None,
            refresh_interval_ms: None,
        }
    }
}

/// Guarda do lock adquirido; libera no `release()` ou no `Drop`.
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
    data: LockData,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    released: bool,
}

impl LockGuard {
    /// Caminho do lockfile.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Dados da geração do lock.
    #[must_use]
    pub fn data(&self) -> &LockData {
        &self.data
    }

    /// Libera o lock (remoção condicional anti-TOCTOU) e para o refresh.
    pub fn release(&mut self) -> Result<(), LockError> {
        self.release_inner();
        Ok(())
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        remove_lock_file(
            &self.path,
            Expected::Generation(Box::new(self.data.clone())),
        );
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Segurança extra do port: um guard esquecido libera o lock (mesma remoção
        // condicional por geração). O oráculo só libera via `release()` explícito.
        self.release_inner();
    }
}

/// Adquire o lock exclusivo.
pub fn acquire(options: &LockOptions) -> Result<LockGuard, LockError> {
    let stale_timeout_ms = options
        .stale_timeout_ms
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_STALE_TIMEOUT_MS);

    let data = LockData {
        pid: std::process::id(),
        lock_id: uuid::Uuid::new_v4().to_string(),
        created_at: now_iso(),
        host: hostname(),
        platform: platform_name(),
    };
    let serialized =
        serde_json::to_string_pretty(&data).map_err(|err| LockError::Io(err.to_string()))?;

    let mut acquired = false;
    for _ in 0..MAX_ACQUIRE_ATTEMPTS {
        let publish = publish_lock_via_link(&options.path, &serialized)?;
        let publish = match publish {
            PublishResult::Created => {
                acquired = true;
                break;
            }
            PublishResult::Unsupported => match publish_lock_via_wx(&options.path, &serialized) {
                Ok(()) => {
                    acquired = true;
                    break;
                }
                Err(PublishError::Exists) => PublishResult::Exists,
                Err(PublishError::Io(message)) => return Err(LockError::Io(message)),
            },
            PublishResult::Exists => PublishResult::Exists,
        };
        debug_assert!(matches!(publish, PublishResult::Exists));

        // Symlink nunca é lock válido: remove apenas o link.
        match std::fs::symlink_metadata(&options.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                remove_lock_file(&options.path, Expected::Simple);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }

        let read = read_existing_lock(&options.path, true);
        let Some(existing) = read.lock.clone() else {
            match read.status {
                ReadStatus::Missing => continue,
                ReadStatus::IoError => {
                    let message = format!(
                        "Não foi possível ler o lockfile \"{}\" (erro de I/O transitório). Tratando como lock ativo por segurança.",
                        options.path.display()
                    );
                    return Err(LockError::Active {
                        message,
                        io_error: true,
                    });
                }
                ReadStatus::Invalid | ReadStatus::Ok => {
                    remove_lock_file(&options.path, Expected::Invalid);
                    if std::fs::symlink_metadata(&options.path).is_ok() {
                        return Err(LockError::Other(format!(
                            "Não foi possível remover o lockfile inválido em \"{}\". O caminho está ocupado por um arquivo/diretório não removível (verifique permissões).",
                            options.path.display()
                        )));
                    }
                    continue;
                }
            }
        };

        let raw_created_at = existing
            .created_at
            .parse::<DateTime<Utc>>()
            .map_or(-1, |parsed| parsed.timestamp_millis());
        let now_ms = Utc::now().timestamp_millis();
        let is_future = raw_created_at > now_ms + CLOCK_SKEW_TOLERANCE_MS;
        let is_invalid = raw_created_at < 0 || is_future;
        let mtime_ms = read.mtime_ms.unwrap_or(0);
        let is_mtime_future = mtime_ms > now_ms + CLOCK_SKEW_TOLERANCE_MS;
        let last_renewed = std::cmp::max(
            if is_invalid { 0 } else { raw_created_at },
            if is_mtime_future { 0 } else { mtime_ms },
        );
        let lock_age = std::cmp::max(0, now_ms - last_renewed);
        let is_stale = lock_age > i64::try_from(stale_timeout_ms).unwrap_or(i64::MAX);

        if is_stale {
            remove_lock_file(&options.path, Expected::Generation(Box::new(existing)));
            continue;
        }

        if existing.host == hostname() {
            let alive = is_process_alive(existing.pid);
            if alive && !options.force {
                return Err(LockError::Active {
                    message: format!(
                        "O processo já está em execução no host local (PID ativo: {}, iniciado em: {}).",
                        existing.pid, existing.created_at
                    ),
                    io_error: false,
                });
            }
            if !alive {
                remove_lock_file(&options.path, Expected::Generation(Box::new(existing)));
                continue;
            }
            remove_lock_file(&options.path, Expected::Generation(Box::new(existing)));
            continue;
        }

        if !options.force {
            return Err(LockError::Active {
                message: format!(
                    "O processo está ativo em outro host ({} , PID: {}, iniciado em: {}).",
                    existing.host, existing.pid, existing.created_at
                ),
                io_error: false,
            });
        }
        remove_lock_file(&options.path, Expected::Generation(Box::new(existing)));
    }

    if !acquired {
        return Err(LockError::Other(format!(
            "Não foi possível adquirir o lockfile \"{}\" após {MAX_ACQUIRE_ATTEMPTS} tentativas concorrentes.",
            options.path.display()
        )));
    }

    let refresh_interval_ms = options
        .refresh_interval_ms
        .filter(|value| *value > 0)
        .unwrap_or_else(|| (stale_timeout_ms / 3).clamp(50, 5 * 60 * 1000));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = spawn_refresh(
        options.path.clone(),
        data.lock_id.clone(),
        data.pid,
        refresh_interval_ms,
        Arc::clone(&stop),
    );

    Ok(LockGuard {
        path: options.path.clone(),
        data,
        stop,
        thread: Some(thread),
        released: false,
    })
}

/// Verifica se um PID está vivo (EPERM conta como vivo).
#[must_use]
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let raw = i32::try_from(pid).unwrap_or(i32::MAX);
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw), None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => true,
            Err(_) => false,
        }
    }
    #[cfg(windows)]
    {
        // `tasklist` é nativo do Windows e não depende de APIs externas.
        use std::process::Command;
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.contains(&pid.to_string())
            }
            // Se o `tasklist` não estiver disponível, assume vivo (fail-safe).
            Err(_) => true,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        // Plataformas sem verificação barata; assume vivo (fail-safe).
        true
    }
}

/// Hostname da máquina.
#[must_use]
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        nix::unistd::gethostname().map_or_else(
            |_| "unknown".to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string())
    }
}

fn platform_name() -> String {
    match std::env::consts::OS {
        "macos" => "darwin".to_string(),
        "windows" => "win32".to_string(),
        other => other.to_string(),
    }
}

fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

enum PublishResult {
    Created,
    Exists,
    Unsupported,
}

enum PublishError {
    Exists,
    Io(String),
}

fn publish_lock_via_link(path: &Path, serialized: &str) -> Result<PublishResult, LockError> {
    let temp = format!(
        "{}.tmp-{}-{}",
        path.to_string_lossy(),
        std::process::id(),
        random_hex(8)
    );
    let temp_path = PathBuf::from(&temp);
    let write_result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path)?;
        std::io::Write::write_all(&mut file, serialized.as_bytes())?;
        let _ = file.sync_all();
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(LockError::Io(err.to_string()));
    }

    let result = match std::fs::hard_link(&temp_path, path) {
        Ok(()) => {
            let _ = safe_chmod_600(path);
            PublishResult::Created
        }
        Err(err) => match err.raw_os_error() {
            Some(code) if code == ERR_EXISTS => PublishResult::Exists,
            Some(code) if LINK_UNSUPPORTED_CODES.contains(&code) => PublishResult::Unsupported,
            _ => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(LockError::Io(err.to_string()));
            }
        },
    };
    let _ = std::fs::remove_file(&temp_path);
    Ok(result)
}

fn publish_lock_via_wx(path: &Path, serialized: &str) -> Result<(), PublishError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.raw_os_error() == Some(ERR_EXISTS) => return Err(PublishError::Exists),
        Err(err) => return Err(PublishError::Io(err.to_string())),
    };
    if let Err(err) = std::io::Write::write_all(&mut file, serialized.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(PublishError::Io(err.to_string()));
    }
    let _ = file.sync_all();
    drop(file);
    let _ = safe_chmod_600(path);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadStatus {
    Ok,
    Missing,
    Invalid,
    IoError,
}

struct ReadOutcome {
    lock: Option<LockData>,
    status: ReadStatus,
    mtime_ms: Option<i64>,
}

fn read_existing_lock(path: &Path, allow_grace: bool) -> ReadOutcome {
    let deadline =
        SystemTime::now() + Duration::from_millis(if allow_grace { LOCK_READ_GRACE_MS } else { 0 });
    let mut io_error = false;

    loop {
        match std::fs::read_to_string(path) {
            Ok(content) if !content.trim().is_empty() => {
                if let Ok(parsed) = serde_json::from_str::<LockData>(&content) {
                    if parsed.pid > 0 {
                        let mtime_ms = metadata_mtime_ms(path);
                        return ReadOutcome {
                            lock: Some(parsed),
                            status: ReadStatus::Ok,
                            mtime_ms,
                        };
                    }
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if SystemTime::now() < deadline {
                    std::thread::sleep(Duration::from_millis(LOCK_READ_RETRY_MS));
                    continue;
                }
                return ReadOutcome {
                    lock: None,
                    status: ReadStatus::Missing,
                    mtime_ms: None,
                };
            }
            Err(err) if err.raw_os_error() == Some(21) => {
                // EISDIR
                return ReadOutcome {
                    lock: None,
                    status: ReadStatus::Invalid,
                    mtime_ms: None,
                };
            }
            Err(_) => io_error = true,
        }

        if SystemTime::now() >= deadline {
            return ReadOutcome {
                lock: None,
                status: if io_error {
                    ReadStatus::IoError
                } else {
                    ReadStatus::Invalid
                },
                mtime_ms: None,
            };
        }
        std::thread::sleep(Duration::from_millis(LOCK_READ_RETRY_MS));
    }
}

fn metadata_mtime_ms(path: &Path) -> Option<i64> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}

/// Expectativa para remoção condicional.
enum Expected {
    /// Remoção simples (symlink, sem geração JSON).
    Simple,
    /// Espera-se conteúdo inválido (só remove se continuar inválido).
    Invalid,
    /// Só remove se a geração ainda bater.
    Generation(Box<LockData>),
}

#[allow(clippy::needless_pass_by_value)]
fn remove_lock_file(path: &Path, expected: Expected) {
    if matches!(expected, Expected::Simple) {
        let _ = std::fs::remove_file(path);
        return;
    }

    let claim = PathBuf::from(format!(
        "{}.tmp-{}-{}",
        path.to_string_lossy(),
        std::process::id(),
        random_hex(8)
    ));
    if std::fs::rename(path, &claim).is_err() {
        return; // já removido/substituído
    }

    let matches = match (&expected, std::fs::read_to_string(&claim)) {
        (Expected::Invalid, Ok(content)) => serde_json::from_str::<LockData>(&content).is_err(),
        (Expected::Invalid, Err(_)) => !claim_is_directory(&claim),
        (Expected::Generation(expected), Ok(content)) => {
            match serde_json::from_str::<LockData>(&content) {
                Ok(current) => {
                    if current.lock_id.is_empty() {
                        current.pid == expected.pid
                            && (expected.created_at.is_empty()
                                || current.created_at == expected.created_at)
                    } else {
                        current.lock_id == expected.lock_id
                    }
                }
                Err(_) => false,
            }
        }
        (Expected::Generation(_), Err(_)) => false,
        (Expected::Simple, _) => true,
    };

    if matches {
        let _ = std::fs::remove_file(&claim);
    } else {
        restore_claim_safely(&claim, path);
    }
}

fn claim_is_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Devolve um claim ao caminho do lock sem sobrescrever outra geração.
fn restore_claim_safely(claim: &Path, target: &Path) -> bool {
    match std::fs::hard_link(claim, target) {
        Ok(()) => {
            let _ = std::fs::remove_file(claim);
            true
        }
        Err(err) if err.raw_os_error() == Some(17) => {
            let _ = std::fs::remove_file(claim);
            false
        }
        Err(_) => {
            if std::fs::symlink_metadata(target).is_ok() {
                let _ = std::fs::remove_file(claim);
                return false;
            }
            let restored = std::fs::rename(claim, target).is_ok();
            if !restored {
                let _ = std::fs::remove_file(claim);
            }
            restored
        }
    }
}

fn spawn_refresh(
    path: PathBuf,
    lock_id: String,
    pid: u32,
    interval_ms: u64,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let interval = Duration::from_millis(interval_ms);
        let mut elapsed = Duration::ZERO;
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
            elapsed += Duration::from_millis(50);
            if elapsed < interval {
                continue;
            }
            elapsed = Duration::ZERO;
            refresh_once(&path, &lock_id, pid);
        }
    })
}

fn refresh_once(path: &Path, lock_id: &str, pid: u32) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(current) = serde_json::from_str::<LockData>(&content) else {
        return;
    };
    let is_ours = if current.lock_id.is_empty() {
        current.pid == pid
    } else {
        current.lock_id == lock_id
    };
    if !is_ours {
        return;
    }

    // Renovação por mtime (atômica, sem reescrever o conteúdo).
    // No Windows, `set_modified` exige um handle com acesso de escrita.
    let mtime_ok = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()))
        .is_ok();
    if mtime_ok {
        return;
    }

    // Fallback: reescrita condicional via claim.
    let claim = PathBuf::from(format!(
        "{}.refresh-{}-{}",
        path.to_string_lossy(),
        std::process::id(),
        random_hex(8)
    ));
    if std::fs::rename(path, &claim).is_err() {
        return;
    }
    let ours = std::fs::read_to_string(&claim)
        .ok()
        .and_then(|content| serde_json::from_str::<LockData>(&content).ok())
        .is_some_and(|current| {
            if current.lock_id.is_empty() {
                current.pid == pid
            } else {
                current.lock_id == lock_id
            }
        });
    if !ours {
        restore_claim_safely(&claim, path);
        return;
    }
    if let Ok(mut updated) =
        serde_json::from_str::<LockData>(&std::fs::read_to_string(&claim).unwrap_or_default())
    {
        updated.created_at = now_iso();
        if let Ok(serialized) = serde_json::to_string_pretty(&updated) {
            let _ = crate::secure_fs::safe_write_file(&claim, serialized.as_bytes());
        }
    }
    restore_claim_safely(&claim, path);
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    let mut out = String::with_capacity(bytes * 2);
    for byte in buffer {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_for(dir: &Path) -> LockOptions {
        LockOptions::new(dir.join("ali-coins-u1000.lock"))
    }

    #[test]
    fn adquire_e_libera() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        let mut guard = acquire(&options).expect("adquiriu");
        assert!(guard.path().exists());
        let data = guard.data().clone();
        assert_eq!(data.pid, std::process::id());
        assert_ne!(data.lock_id, "");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(guard.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        guard.release().expect("release");
        assert!(!options.path.exists());
    }

    #[test]
    fn segundo_acquire_no_mesmo_processo_da_lock_active() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        let _guard = acquire(&options).expect("adquiriu");
        let err = acquire(&options).expect_err("deveria bloquear");
        assert!(err.is_lock_active());
        assert!(err.to_string().contains("PID ativo"));
    }

    #[test]
    fn force_sobrescreve_lock_ativo() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        let guard = acquire(&options).expect("adquiriu");
        let mut forced = options.clone();
        forced.force = true;
        let guard2 = acquire(&forced).expect("force");
        assert_ne!(guard.data().lock_id, guard2.data().lock_id);
        drop(guard); // não deve remover o lock da nova geração
        assert!(options.path.exists());
    }

    #[test]
    fn remove_lock_stale_de_pid_morto() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        let stale = LockData {
            pid: 999_999_999,
            lock_id: "geracao-antiga".to_string(),
            created_at: (Utc::now() - chrono::Duration::hours(2))
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string(),
            host: hostname(),
            platform: platform_name(),
        };
        std::fs::write(&options.path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();
        let guard = acquire(&options).expect("removeu stale");
        assert_ne!(guard.data().lock_id, "geracao-antiga");
    }

    #[test]
    fn remove_lock_invalido_apos_carencia() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        std::fs::write(&options.path, "{json quebrado").unwrap();
        let guard = acquire(&options).expect("removeu inválido");
        assert!(guard.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn remove_symlink_suspeito() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        let target = dir.path().join("alvo");
        std::fs::write(&target, "x").unwrap();
        std::os::unix::fs::symlink(&target, &options.path).unwrap();
        let guard = acquire(&options).expect("removeu symlink");
        assert!(guard.path().exists());
    }

    #[test]
    fn refresh_renova_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut options = options_for(dir.path());
        options.stale_timeout_ms = Some(300); // refresh a cada 100ms
        let guard = acquire(&options).expect("adquiriu");
        std::thread::sleep(std::time::Duration::from_millis(450));
        // Tolera atraso do refresh (o fallback por claim tem janelas curtas
        // em que o arquivo pode estar momentaneamente ausente no Windows).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut last_age = i64::MAX;
        while std::time::Instant::now() < deadline {
            if let Some(mtime) = metadata_mtime_ms(guard.path()) {
                last_age = Utc::now().timestamp_millis() - mtime;
                if last_age < 300 {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("mtime não foi renovado (idade={last_age}ms)");
    }

    #[test]
    fn recreate_apos_drop() {
        let dir = tempfile::tempdir().unwrap();
        let options = options_for(dir.path());
        {
            let _guard = acquire(&options).expect("adquiriu");
        }
        let _guard2 = acquire(&options).expect("readquiriu após drop");
    }
}
