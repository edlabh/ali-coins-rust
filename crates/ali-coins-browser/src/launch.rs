//! Decisões de launch do Chromium compatíveis com `browser.js` do oráculo.
//!
//! Cobre a cascata de perfis, flags de baixo consumo, `--no-sandbox`
//! condicional, `--disable-dev-shm-usage`, sanitização do ambiente repassado ao
//! navegador e o perfil de emulação mobile (Pixel 7).

use ali_coins_core::config::EnvSource;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Heap padrão do V8 no Chromium (MB).
pub const DEFAULT_JS_HEAP_MB: u64 = 128;
/// Piso do heap configurável.
pub const MIN_JS_HEAP_MB: u64 = 64;
/// Teto do heap configurável.
pub const MAX_JS_HEAP_MB: u64 = 2048;
/// Limite de `/dev/shm` (MB) abaixo do qual usamos disco.
pub const DEV_SHM_MIN_FREE_MB: u64 = 128;

/// Flags de economia de CPU/rede aplicadas sempre (sem `BackForwardCache`).
pub const BACKGROUND_CPU_SAVING_ARGS: [&str; 17] = [
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-breakpad",
    "--mute-audio",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-default-apps",
    "--disable-client-side-phishing-detection",
    "--metrics-recording-only",
    "--disable-animations",
    "--disable-smooth-scrolling",
    "--disable-features=Translate,AcceptCHFrame,MediaRouter,OptimizationHints",
    "--no-pings",
    "--disable-extensions",
    "--disable-notifications",
    "--prerender=disabled",
];

/// Estado da decisão de sandbox.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSandboxInfo {
    /// Processo roda como root.
    pub is_root: bool,
    /// Ambiente de CI.
    pub is_ci: bool,
    /// `NO_SANDBOX=true|1`.
    pub is_explicit: bool,
    /// Deve desabilitar o sandbox?
    pub should_disable: bool,
}

/// Decide o sandbox conforme root/CI/`NO_SANDBOX`.
#[must_use]
pub fn no_sandbox_required(env: &EnvSource, is_root: bool) -> NoSandboxInfo {
    let is_ci = env.get("CI").is_some_and(|value| !value.is_empty());
    let is_explicit = ali_coins_core::config::env::bool_true_exact(env.get("NO_SANDBOX"));
    NoSandboxInfo {
        is_root,
        is_ci,
        is_explicit,
        should_disable: is_root || is_ci || is_explicit,
    }
}

/// Heap do V8 configurável (`CHROMIUM_JS_HEAP_MB`, clamp 64–2048).
#[must_use]
pub fn chromium_js_heap_mb(env: &EnvSource) -> u64 {
    let Some(raw) = env
        .get("CHROMIUM_JS_HEAP_MB")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
    else {
        return DEFAULT_JS_HEAP_MB;
    };
    #[allow(clippy::cast_sign_loss)]
    let raw = raw as u64;
    raw.clamp(MIN_JS_HEAP_MB, MAX_JS_HEAP_MB)
}

/// Modo de baixo consumo (padrão ligado; `0/false/off/no` desligam).
#[must_use]
pub fn low_memory_mode_enabled(env: &EnvSource) -> bool {
    !matches!(
        env.get("CHROMIUM_LOW_MEMORY")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// Bloqueio de service workers (padrão ligado).
#[must_use]
pub fn service_worker_blocking_enabled(env: &EnvSource) -> bool {
    !matches!(
        env.get("PW_BLOCK_SERVICE_WORKERS")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// `/dev/shm` pequeno? (`None` = sem acesso → fallback defensivo)
#[must_use]
pub fn should_disable_dev_shm_usage(platform: &str, shm_free_mb: Option<u64>) -> bool {
    if platform != "linux" {
        return false;
    }
    shm_free_mb.is_none_or(|free| free < DEV_SHM_MIN_FREE_MB)
}

/// Flags de baixo consumo (heap dinâmico).
#[must_use]
pub fn low_memory_chromium_args(heap_mb: u64) -> Vec<String> {
    vec![
        "--disable-gpu".to_string(),
        "--disable-software-rasterizer".to_string(),
        "--renderer-process-limit=1".to_string(),
        format!("--js-flags=--max-old-space-size={heap_mb}"),
        "--disk-cache-size=10485760".to_string(),
    ]
}

/// Entrada de `buildChromiumArgs`.
#[derive(Debug, Clone)]
pub struct ChromiumArgsInput<'a> {
    /// Ambiente (flags condicionais).
    pub env: &'a EnvSource,
    /// Processo é root.
    pub is_root: bool,
    /// `/dev/shm` pequeno.
    pub dev_shm_small: bool,
    /// Força `--no-sandbox` (fallback da cascata).
    pub force_no_sandbox: bool,
    /// Sobrescreve a decisão de baixo consumo.
    pub low_memory: Option<bool>,
}

/// Monta os argumentos do Chromium na mesma ordem do oráculo.
#[must_use]
pub fn build_chromium_args(input: &ChromiumArgsInput<'_>) -> Vec<String> {
    let info = no_sandbox_required(input.env, input.is_root);
    let should_disable = info.should_disable || input.force_no_sandbox;
    let use_low_memory = input
        .low_memory
        .unwrap_or_else(|| low_memory_mode_enabled(input.env));

    let mut args = vec!["--disable-blink-features=AutomationControlled".to_string()];
    args.extend(BACKGROUND_CPU_SAVING_ARGS.iter().map(ToString::to_string));
    if input.dev_shm_small {
        args.push("--disable-dev-shm-usage".to_string());
    }
    if should_disable {
        args.push("--no-sandbox".to_string());
        args.push("--disable-setuid-sandbox".to_string());
    }
    if use_low_memory {
        args.extend(low_memory_chromium_args(chromium_js_heap_mb(input.env)));
        if should_disable {
            args.push("--no-zygote".to_string());
        }
    }
    args
}

/// Perfil de tentativa da cascata de launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchProfile {
    /// Usar flags de baixo consumo.
    pub low_memory: bool,
    /// Forçar `--no-sandbox` na tentativa.
    pub force_no_sandbox: bool,
    /// A tentativa só vale se o erro anterior for de sandbox/zygote.
    pub requires_sandbox_error: bool,
}

/// Cascata de perfis: (low,sandbox) → (low,noSandbox*) → (noLow,sandbox) → (noLow,noSandbox*).
#[must_use]
pub fn launch_profiles(low_memory_enabled: bool) -> Vec<LaunchProfile> {
    let mut profiles = Vec::new();
    if low_memory_enabled {
        profiles.push(LaunchProfile {
            low_memory: true,
            force_no_sandbox: false,
            requires_sandbox_error: false,
        });
        profiles.push(LaunchProfile {
            low_memory: true,
            force_no_sandbox: true,
            requires_sandbox_error: true,
        });
    }
    profiles.push(LaunchProfile {
        low_memory: false,
        force_no_sandbox: false,
        requires_sandbox_error: false,
    });
    profiles.push(LaunchProfile {
        low_memory: false,
        force_no_sandbox: true,
        requires_sandbox_error: true,
    });
    profiles
}

fn sensitive_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(secret|password|passwd|\bpass\b|passphrase|_pw\b|pwd|\bsenha\b|token|\bbearer\b|cookie|credential|authorization|api[_-]?key|private[_-]?key)",
        )
        .expect("regex válida")
    })
}

fn sensitive_prefix_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)^(TELEGRAM_|NOTIFY_|ALI_|SESSION_|GITHUB_|HEARTBEAT_)")
            .expect("regex válida")
    })
}

/// Variáveis de proxy que têm credenciais removidas antes do repasse.
pub const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// Remove userinfo de uma URL de proxy (ex.: `http://user:pass@proxy:8080`).
#[must_use]
pub fn strip_url_credentials(raw: &str) -> String {
    if !raw.contains('@') {
        return raw.to_string();
    }
    let has_scheme = Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://")
        .expect("regex")
        .is_match(raw);
    let candidate = if has_scheme {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let Ok(mut url) = url::Url::parse(&candidate) else {
        return raw.to_string();
    };
    if url.username().is_empty() && url.password().is_none() {
        return raw.to_string();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let mut without = url.to_string().trim_end_matches('/').to_string();
    if !has_scheme {
        without = Regex::new(r"(?i)^https?://")
            .expect("regex")
            .replace(&without, "")
            .to_string();
    }
    without
}

/// Ambiente sanitizado para o Chromium (sem segredos; proxies sem credenciais).
#[must_use]
pub fn sanitize_env(entries: &[(String, String)]) -> Vec<(String, String)> {
    entries
        .iter()
        .filter(|(key, _)| {
            !sensitive_key_regex().is_match(key) && !sensitive_prefix_regex().is_match(key)
        })
        .map(|(key, value)| {
            if PROXY_ENV_KEYS.contains(&key.as_str()) {
                (key.clone(), strip_url_credentials(value))
            } else {
                (key.clone(), value.clone())
            }
        })
        .collect()
}

/// Perfil de emulação mobile (Playwright `devices['Pixel 7']`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProfile {
    /// User agent.
    pub user_agent: String,
    /// Viewport em CSS pixels.
    pub viewport: Viewport,
    /// Device scale factor.
    pub device_scale_factor: f64,
    /// `isMobile`.
    pub is_mobile: bool,
    /// `hasTouch`.
    pub has_touch: bool,
    /// Locale aplicado pelo oráculo.
    pub locale: String,
}

/// Viewport do device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    /// Largura.
    pub width: u32,
    /// Altura.
    pub height: u32,
}

/// Perfil Pixel 7 usado pelo oráculo (valores validados por fixture).
#[must_use]
pub fn pixel7_profile() -> DeviceProfile {
    DeviceProfile {
        user_agent: "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.8010.12 Mobile Safari/537.36".to_string(),
        viewport: Viewport {
            width: 412,
            height: 839,
        },
        device_scale_factor: 2.625,
        is_mobile: true,
        has_touch: true,
        locale: "pt-BR".to_string(),
    }
}

/// Hosts de telemetria bloqueados por padrão.
const TELEMETRY_HOSTS: [&str; 8] = [
    "umeng",
    "google-analytics",
    "googletagmanager",
    "doubleclick",
    "facebook.net",
    "tiktok",
    "criteo",
    "bing",
];

/// Recursos bloqueados por padrão (imagem/mídia/fonte) e hosts de telemetria.
#[must_use]
pub fn should_block_resource(resource_type: &str, url: &str, allow_media: bool) -> bool {
    if allow_media {
        return false;
    }
    if matches!(resource_type, "image" | "media" | "font") {
        return true;
    }
    let lower = url.to_lowercase();
    TELEMETRY_HOSTS.iter().any(|host| lower.contains(host))
}

/// Caminhos relativos do binário do Chromium dentro de um diretório
/// `chromium-<versão>` do cache do Playwright, por sistema operacional.
#[must_use]
pub fn host_chromium_rel_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[&str] = &[
        "chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium",
        "chrome-mac-x64/Chromium.app/Contents/MacOS/Chromium",
        "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
    ];
    #[cfg(target_os = "windows")]
    const CANDIDATES: &[&str] = &["chrome-win64/chrome.exe", "chrome-win/chrome.exe"];
    #[cfg(all(unix, not(target_os = "macos")))]
    const CANDIDATES: &[&str] = &["chrome-linux64/chrome", "chrome-linux/chrome"];
    CANDIDATES
}

/// Subdiretório padrão do cache do Playwright por SO.
fn playwright_cache_rel() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Library/Caches/ms-playwright"
    }
    #[cfg(target_os = "windows")]
    {
        "AppData/Local/ms-playwright"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        ".cache/ms-playwright"
    }
}

/// Base do cache do Playwright: `PLAYWRIGHT_BROWSERS_PATH` ou o padrão do SO.
fn playwright_cache_base(env: &EnvSource) -> Option<std::path::PathBuf> {
    if let Some(value) = env.get("PLAYWRIGHT_BROWSERS_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
    }
    let home = env
        .get("HOME")
        .or_else(|| env.get("USERPROFILE"))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(std::path::Path::new(home).join(playwright_cache_rel()))
}

/// Procura o Chromium de maior versão no cache (`chromium-*`).
fn chromium_in_base(base: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut versions: Vec<(u64, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix("chromium-") else {
            continue;
        };
        let Ok(version) = rest.parse::<u64>() else {
            continue;
        };
        if entry.path().is_dir() {
            versions.push((version, entry.path()));
        }
    }
    versions.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, dir) in versions {
        for rel in host_chromium_rel_candidates() {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve o binário do Chromium (equivalente à descoberta do Playwright):
/// `ALI_COINS_CHROME` explícito → cache do Playwright (`PLAYWRIGHT_BROWSERS_PATH`
/// ou padrão por SO, maior versão `chromium-*`) → `None` (o driver usa a
/// detecção nativa do chromiumoxide: PATH, registro do Windows, instalações usuais).
#[must_use]
pub fn resolve_chromium_path(env: &EnvSource) -> Option<std::path::PathBuf> {
    if let Some(value) = env.get("ALI_COINS_CHROME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            // Explícito vence mesmo se o caminho não existir (erro claro no launch).
            return Some(std::path::PathBuf::from(trimmed));
        }
    }
    playwright_cache_base(env).and_then(|base| chromium_in_base(&base))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn args_basicos_com_low_memory() {
        let env = env(&[]);
        let args = build_chromium_args(&ChromiumArgsInput {
            env: &env,
            is_root: false,
            dev_shm_small: false,
            force_no_sandbox: false,
            low_memory: None,
        });
        assert_eq!(args[0], "--disable-blink-features=AutomationControlled");
        assert!(args.iter().any(|arg| arg == "--disable-gpu"));
        assert!(
            args.iter()
                .any(|arg| arg == "--js-flags=--max-old-space-size=128")
        );
        assert!(!args.iter().any(|arg| arg == "--no-sandbox"));
        // BackForwardCache NÃO é desabilitado.
        assert!(!args.iter().any(|arg| arg.contains("BackForwardCache")));
    }

    #[test]
    fn no_sandbox_por_root_ci_e_env() {
        let base = env(&[]);
        assert!(no_sandbox_required(&base, true).should_disable);
        assert!(no_sandbox_required(&env(&[("CI", "true")]), false).should_disable);
        assert!(no_sandbox_required(&env(&[("NO_SANDBOX", "1")]), false).should_disable);
        assert!(!no_sandbox_required(&base, false).should_disable);

        // Com sandbox desabilitado + low memory, `--no-zygote` entra.
        let args = build_chromium_args(&ChromiumArgsInput {
            env: &base,
            is_root: true,
            dev_shm_small: true,
            force_no_sandbox: false,
            low_memory: None,
        });
        assert!(args.iter().any(|arg| arg == "--no-sandbox"));
        assert!(args.iter().any(|arg| arg == "--disable-setuid-sandbox"));
        assert!(args.iter().any(|arg| arg == "--no-zygote"));
        assert!(args.iter().any(|arg| arg == "--disable-dev-shm-usage"));
    }

    #[test]
    fn low_memory_desligado_e_heap_configuravel() {
        let env = env(&[
            ("CHROMIUM_LOW_MEMORY", "false"),
            ("CHROMIUM_JS_HEAP_MB", "9999"),
        ]);
        let args = build_chromium_args(&ChromiumArgsInput {
            env: &env,
            is_root: false,
            dev_shm_small: false,
            force_no_sandbox: false,
            low_memory: None,
        });
        assert!(!args.iter().any(|arg| arg == "--disable-gpu"));
        assert_eq!(chromium_js_heap_mb(&env), MAX_JS_HEAP_MB);
    }

    #[test]
    fn cascata_de_perfis() {
        let profiles = launch_profiles(true);
        assert_eq!(profiles.len(), 4);
        assert!(profiles[1].force_no_sandbox && profiles[1].requires_sandbox_error);
        assert!(profiles[3].force_no_sandbox && profiles[3].requires_sandbox_error);
        assert!(!profiles[2].low_memory);
        let profiles = launch_profiles(false);
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|profile| !profile.low_memory));
    }

    #[test]
    fn sanitiza_ambiente_removendo_segredos() {
        let entries = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("ALI_PASSWORD".to_string(), "segredo".to_string()),
            ("SESSION_SECRET".to_string(), "segredo".to_string()),
            ("TELEGRAM_BOT_TOKEN".to_string(), "segredo".to_string()),
            ("MY_API_KEY".to_string(), "segredo".to_string()),
            (
                "HTTP_PROXY".to_string(),
                "http://user:pass@proxy:8080".to_string(),
            ),
            ("LANG".to_string(), "C.UTF-8".to_string()),
        ];
        let sanitized = sanitize_env(&entries);
        let keys: Vec<&str> = sanitized.iter().map(|(key, _)| key.as_str()).collect();
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"LANG"));
        assert!(!keys.contains(&"ALI_PASSWORD"));
        assert!(!keys.contains(&"SESSION_SECRET"));
        assert!(!keys.contains(&"TELEGRAM_BOT_TOKEN"));
        assert!(!keys.contains(&"MY_API_KEY"));
        let proxy = sanitized
            .iter()
            .find(|(key, _)| key == "HTTP_PROXY")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert!(!proxy.contains("user:pass"));
    }

    #[test]
    fn bloqueio_de_recursos() {
        assert!(should_block_resource("image", "https://x/y.png", false));
        assert!(should_block_resource(
            "script",
            "https://www.google-analytics.com/x.js",
            false
        ));
        assert!(!should_block_resource("image", "https://x/y.png", true));
        assert!(!should_block_resource(
            "document",
            "https://m.aliexpress.com/",
            false
        ));
    }

    #[test]
    fn dev_shm() {
        assert!(!should_disable_dev_shm_usage("darwin", Some(1)));
        assert!(should_disable_dev_shm_usage("linux", Some(64)));
        assert!(!should_disable_dev_shm_usage("linux", Some(256)));
        assert!(should_disable_dev_shm_usage("linux", None));
    }

    #[test]
    fn resolve_chromium_prefere_caminho_explicito() {
        let explicito = env(&[("ALI_COINS_CHROME", "/x/chrome")]);
        assert_eq!(
            resolve_chromium_path(&explicito),
            Some(std::path::PathBuf::from("/x/chrome"))
        );
        // Explícito vence mesmo se não existir (erro claro no launch).
        assert_eq!(
            resolve_chromium_path(&env(&[("ALI_COINS_CHROME", "/nao/existe")])),
            Some(std::path::PathBuf::from("/nao/existe"))
        );
    }

    #[test]
    fn resolve_chromium_pega_maior_versao_do_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        for version in ["chromium-100", "chromium-123"] {
            std::fs::create_dir_all(dir.path().join(version)).expect("dir");
        }
        let rel = host_chromium_rel_candidates()[0];
        let bin = dir.path().join("chromium-123").join(rel);
        std::fs::create_dir_all(bin.parent().expect("parent")).expect("dirs");
        std::fs::write(&bin, b"fake").expect("bin");

        let base = dir.path().to_str().expect("utf-8");
        let cache_env = env(&[("PLAYWRIGHT_BROWSERS_PATH", base)]);
        assert_eq!(resolve_chromium_path(&cache_env), Some(bin));

        // `PLAYWRIGHT_BROWSERS_PATH` vazio cai no padrão do SO (HOME/USERPROFILE).
        let home_env = env(&[("PLAYWRIGHT_BROWSERS_PATH", "  "), ("HOME", base)]);
        // O padrão do SO aponta para `<HOME>/<subdir do SO>`; aqui só garantimos
        // que a resolução não falha (o layout de teste não existe nesse caminho).
        let _ = resolve_chromium_path(&home_env);
    }
}
