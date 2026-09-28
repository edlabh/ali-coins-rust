//! Resolução dos caminhos de sessão (equivalente a `resolveSessionPaths`).

use super::SessionOptions;
use std::path::{Path, PathBuf};

/// Caminhos resolvidos da sessão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPaths {
    /// Sessão em texto puro.
    pub s_path: PathBuf,
    /// Sessão cifrada.
    pub enc_path: PathBuf,
    /// Metadados.
    pub m_path: PathBuf,
    /// Diretório de scratch/backups.
    pub scratch_dir: PathBuf,
}

/// Resolve paths a partir das opções e do diretório base padrão.
#[must_use]
pub fn resolve_session_paths(options: &SessionOptions, default_base: &Path) -> SessionPaths {
    let base_dir = options
        .base_dir
        .clone()
        .unwrap_or_else(|| default_base.to_path_buf());
    let raw_path = options
        .session_path
        .clone()
        .unwrap_or_else(|| base_dir.join("session.json"));

    let raw_str = raw_path.to_string_lossy();
    let (plain_path, enc_path) = if raw_str.ends_with(".enc") {
        let plain = PathBuf::from(raw_str.trim_end_matches(".enc"));
        (plain, raw_path.clone())
    } else {
        let mut enc = raw_path.clone().into_os_string();
        enc.push(".enc");
        (raw_path.clone(), PathBuf::from(enc))
    };

    let m_path = if let Some(meta) = &options.session_meta_path {
        meta.clone()
    } else if options.session_path.is_some() {
        let dir = plain_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let base = plain_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Some(rest) = base.strip_prefix("session_") {
            dir.join(format!("session_meta_{rest}"))
        } else if base == "session.json" {
            dir.join("session_meta.json")
        } else {
            let stem = base.strip_suffix(".json").unwrap_or(&base);
            dir.join(format!("{stem}_meta.json"))
        }
    } else {
        base_dir.join("session_meta.json")
    };

    let scratch_dir = options
        .scratch_dir
        .clone()
        .unwrap_or_else(|| base_dir.join("scratch"));

    SessionPaths {
        s_path: plain_path,
        enc_path,
        m_path,
        scratch_dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caminhos_padrao() {
        let options = SessionOptions::with_base_dir(PathBuf::from("/proj"));
        let paths = resolve_session_paths(&options, Path::new("/proj"));
        assert_eq!(paths.s_path, PathBuf::from("/proj/session.json"));
        assert_eq!(paths.enc_path, PathBuf::from("/proj/session.json.enc"));
        assert_eq!(paths.m_path, PathBuf::from("/proj/session_meta.json"));
        assert_eq!(paths.scratch_dir, PathBuf::from("/proj/scratch"));
    }

    #[test]
    fn caminho_enc_reverso() {
        let mut options = SessionOptions::with_base_dir(PathBuf::from("/proj"));
        options.session_path = Some(PathBuf::from("/proj/session_abcd1234.json.enc"));
        let paths = resolve_session_paths(&options, Path::new("/proj"));
        assert_eq!(paths.s_path, PathBuf::from("/proj/session_abcd1234.json"));
        assert_eq!(
            paths.enc_path,
            PathBuf::from("/proj/session_abcd1234.json.enc")
        );
        assert_eq!(
            paths.m_path,
            PathBuf::from("/proj/session_meta_abcd1234.json")
        );
    }

    #[test]
    fn caminho_customizado_nao_session() {
        let mut options = SessionOptions::with_base_dir(PathBuf::from("/proj"));
        options.session_path = Some(PathBuf::from("/proj/minha-sessao.json"));
        let paths = resolve_session_paths(&options, Path::new("/proj"));
        assert_eq!(paths.m_path, PathBuf::from("/proj/minha-sessao_meta.json"));
    }
}
