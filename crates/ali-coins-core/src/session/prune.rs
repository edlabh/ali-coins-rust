//! Poda de backups/artefatos em `scratch/` (equivalente a `pruneSessionBackups`).

use super::{SessionOptions, resolve_session_paths};
use crate::config::EnvSource;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Arquivos que NUNCA são removidos pela política de retenção.
#[must_use]
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn is_prunable_artifact(file: &str) -> bool {
    if file == "session.json"
        || file == "session.json.enc"
        || file.starts_with("session.json")
        || file == "session_meta.json"
        || file.starts_with("session_meta")
        || file == "session_token.txt"
        || file == "cron.log"
        || file.starts_with("cron.log")
        || file.starts_with("credentials.env")
        || file == ".gitkeep"
    {
        return false;
    }
    if file.starts_with("session.bak-") && (file.ends_with(".json.enc") || file.ends_with(".json"))
    {
        return true;
    }
    if file.ends_with(".zip") && file.contains("-trace-") {
        return true;
    }
    if file.ends_with(".png") || file.ends_with(".jpeg") || file.ends_with(".jpg") {
        return true;
    }
    if (file.starts_with("dom-") && file.ends_with(".hash.txt")) || file == "mobile_body.html" {
        return true;
    }
    if file.contains(".tmp-") {
        return true;
    }
    false
}

/// Remove artefatos antigos dentro de `scratch/`, respeitando a barreira de diretório.
#[must_use]
pub fn prune_session_backups(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
) -> Vec<PathBuf> {
    let paths = resolve_session_paths(options, default_base);
    let target = options
        .scratch_dir
        .clone()
        .unwrap_or_else(|| paths.scratch_dir.clone());
    let allowed_root = options
        .base_dir
        .clone()
        .unwrap_or_else(|| default_base.to_path_buf())
        .join("scratch");
    if !is_within(&allowed_root, &target) {
        return Vec::new();
    }

    let env_days = env
        .get("DIAGNOSTICS_RETENTION_DAYS")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            env.get("SESSION_BACKUP_RETENTION_DAYS")
                .filter(|value| !value.is_empty())
        });
    let retention_days = options.retention_days.unwrap_or_else(|| {
        env_days
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|days| *days > 0.0)
            .unwrap_or(7.0)
    });
    let max_age = Duration::from_secs_f64(retention_days * 24.0 * 60.0 * 60.0);

    let mut pruned = Vec::new();
    let Ok(entries) = std::fs::read_dir(&target) else {
        return pruned;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_prunable_artifact(&name) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if age > max_age {
            let removed = if options.dry_run {
                true
            } else {
                std::fs::remove_file(entry.path()).is_ok()
            };
            if removed {
                pruned.push(entry.path());
            }
        }
    }

    // Temporários órfãos do diretório de sessão (somente com alvo explícito).
    if options.session_path.is_some() || options.session_meta_path.is_some() {
        if let Some(parent) = paths.enc_path.parent() {
            pruned.extend(clean_orphan_tmp_files(parent, 300));
        }
    }

    pruned
}

/// Remove temporários `.tmp-*` mais antigos que `max_age_seconds`.
#[must_use]
pub fn clean_orphan_tmp_files(dir: &Path, max_age_seconds: u64) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return removed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.contains(".tmp-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if age > Duration::from_secs(max_age_seconds) && std::fs::remove_file(entry.path()).is_ok()
        {
            removed.push(entry.path());
        }
    }
    removed
}

/// `target` está dentro de `root`? (comparação lexical segura)
pub(crate) fn is_within(root: &Path, target: &Path) -> bool {
    let root = normalize(root);
    let target = normalize(target);
    let rel = pathdiff(&root, &target);
    match rel {
        Some(rel) => !rel.starts_with(".."),
        None => false,
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Caminho relativo entre dois caminhos absolutos normalizados.
fn pathdiff(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let common = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..base_components.len() {
        out.push("..");
    }
    for component in &target_components[common..] {
        out.push(component.as_os_str());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protegidos_nunca_sao_podados() {
        for file in [
            "session.json",
            "session.json.enc",
            "session.json.tmp-123-abcd",
            "session_meta.json",
            "session_meta_abcd1234.json",
            "session_token.txt",
            "cron.log",
            "credentials.env",
            ".gitkeep",
        ] {
            assert!(
                !is_prunable_artifact(file),
                "{file} não deveria ser podável"
            );
        }
    }

    #[test]
    fn artefatos_podaveis() {
        for file in [
            "session.bak-2026-09-28T12-00-00-000Z.json.enc",
            "session.bak-conta1-2026-09-28T12-00-00-000Z.json",
            "checkin-trace-1.zip",
            "screenshot.png",
            "dom-abc.hash.txt",
            "mobile_body.html",
            "relatorio.tmp-123-abcd",
        ] {
            assert!(is_prunable_artifact(file), "{file} deveria ser podável");
        }
    }

    #[test]
    fn barreira_de_diretorio() {
        assert!(is_within(
            Path::new("/proj/scratch"),
            Path::new("/proj/scratch")
        ));
        assert!(is_within(
            Path::new("/proj/scratch"),
            Path::new("/proj/scratch/sub")
        ));
        assert!(!is_within(Path::new("/proj/scratch"), Path::new("/proj")));
        assert!(!is_within(
            Path::new("/proj/scratch"),
            Path::new("/outro/scratch")
        ));
        assert!(!is_within(
            Path::new("/proj/scratch"),
            Path::new("/proj/scratch/../..")
        ));
    }

    #[test]
    fn poda_remove_artefato_antigo_e_preserva_recente() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scratch = dir.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let old = scratch.join("session.bak-2020-01-01T00-00-00-000Z.json.enc");
        let recent = scratch.join("session.bak-2030-01-01T00-00-00-000Z.json");
        let protected = scratch.join("session.json");
        std::fs::write(&old, "x").unwrap();
        std::fs::write(&recent, "x").unwrap();
        std::fs::write(&protected, "x").unwrap();
        // Envelhece o arquivo antigo.
        let old_time = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        set_mtime(&old, old_time);

        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        let env = EnvSource::default();
        let removed = prune_session_backups(&options, dir.path(), &env);
        assert_eq!(removed.len(), 1);
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(protected.exists());
    }

    fn set_mtime(path: &Path, time: SystemTime) {
        // No Windows, alterar o mtime exige um handle com acesso de escrita.
        let ft = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        ft.set_modified(time).unwrap();
    }
}
