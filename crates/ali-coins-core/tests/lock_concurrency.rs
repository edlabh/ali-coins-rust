//! Corrida real entre processos pelo lockfile (equivalente ao teste do oráculo).
//!
//! O teste `apenas_um_processo_adquire` lança 6 cópias deste binário; cada uma
//! executa `lock_helper_child`, que tenta adquirir o lock e dorme 300 ms.

use ali_coins_core::lock::{LockOptions, acquire};
use std::path::PathBuf;

#[test]
fn lock_helper_child() {
    let Ok(path) = std::env::var("LOCK_HELPER_PATH") else {
        return;
    };
    let options = LockOptions::new(PathBuf::from(path));
    match acquire(&options) {
        Ok(mut guard) => {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = guard.release();
            std::process::exit(0);
        }
        Err(err) if err.is_lock_active() => std::process::exit(3),
        Err(_) => std::process::exit(1),
    }
}

#[test]
fn apenas_um_processo_adquire() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = dir.path().join("ali-coins-u1000.lock");
    let exe = std::env::current_exe().expect("current_exe");

    let mut children = Vec::new();
    for _ in 0..6 {
        let child = std::process::Command::new(&exe)
            .args(["--exact", "lock_helper_child", "--nocapture"])
            .env("LOCK_HELPER_PATH", &lock_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn filho");
        children.push(child);
    }

    let mut success = 0;
    let mut active = 0;
    for mut child in children {
        let status = child.wait().expect("wait");
        match status.code() {
            Some(0) => success += 1,
            Some(3) => active += 1,
            code => panic!("exit inesperado: {code:?}"),
        }
    }

    assert_eq!(success, 1, "exatamente um processo deve adquirir o lock");
    assert_eq!(active, 5, "os demais devem sair com LOCK_ACTIVE (3)");
    assert!(!lock_path.exists(), "o lock deve ser liberado no final");
}
