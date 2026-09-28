//! Escrita atômica de arquivos sensíveis (equivalente a `safeWriteFile` do oráculo).
//!
//! Garantias: arquivo final com modo `0o600`, escrita em temporário no mesmo
//! diretório, `fsync` do arquivo, `rename` atômico e limpeza do temporário em
//! caso de falha.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process;

use rand::RngCore as _;

/// Nome do temporário no mesmo formato do oráculo: `<nome>.tmp-<pid>-<hex>`.
fn temp_path_for(path: &Path) -> io::Result<std::path::PathBuf> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "caminho sem nome de arquivo")
    })?;
    let mut suffix = [0_u8; 8];
    rand::rng().fill_bytes(&mut suffix);
    let mut hex = String::with_capacity(16);
    for byte in &suffix {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(dir.join(format!(
        "{}.tmp-{}-{hex}",
        name.to_string_lossy(),
        process::id()
    )))
}

/// Escreve `data` de forma atômica e com modo `0o600`.
pub fn safe_write_file(path: &Path, data: &[u8]) -> io::Result<()> {
    let temp = temp_path_for(path)?;
    let result = write_and_replace(&temp, path, data);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_and_replace(temp: &Path, dest: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = create_private(temp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, dest)?;
    // Best-effort: persiste o rename no diretório.
    if let Some(dir) = dest.parent() {
        if let Ok(handle) = File::open(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        }) {
            let _ = handle.sync_all();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Ajusta o modo para `0o600` apenas em arquivo regular (não segue symlink).
pub fn safe_chmod_600(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
    }
    Ok(())
}

/// Cria um diretório de saída com `0700` (dump de diagnóstico).
pub fn prepare_output_dir_for_dump(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escreve_e_sobrescreve_com_modo_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("session.json");
        safe_write_file(&file, b"primeiro").expect("write");
        assert_eq!(fs::read(&file).expect("read"), b"primeiro");
        safe_write_file(&file, b"segundo").expect("overwrite");
        assert_eq!(fs::read(&file).expect("read"), b"segundo");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&file).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "modo={mode:o}");
        }
    }

    #[test]
    fn nao_deixa_temporarios() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("dados.bin");
        safe_write_file(&file, b"x").expect("write");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temporários: {leftovers:?}");
    }
}
