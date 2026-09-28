# ADR-0001: Driver de browser — CDP via `chromiumoxide` com trait de abstração

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

O projeto de origem usa Playwright 1.63 (Node) com emulação mobile Pixel 7, `storageState`, `addInitScript`, bloqueio de requisições, trace/screenshot/vídeo, múltiplas páginas/popups e seletores com `:has-text`. O port precisa reproduzir esses comportamentos em Rust sem depender de Node em produção.

## Decision Drivers

- Paridade comportamental com o oráculo (especialmente emulação mobile e interceptação).
- Manutenção ativa e compatibilidade com `tokio`.
- Controle fino para implementar os helpers do Playwright que não existem prontos.
- Custo de migração e risco de dependência abandonada.

## Considered Options

### Opção 1: `chromiumoxide` 0.9.1 (CDP, tokio) — **escolhida**
- Prós: manutenção ativa (publicada em 2026-02), 4,5M downloads, async nativo, cobre `Runtime.evaluate`, `Page`, `Fetch`/`Network`, `Emulation`, `Tracing`, `Target`.
- Contras: API de baixo nível; exige implementar auto-wait/visibilidade, matcher de texto, storage state, presets de device e diálogo.

### Opção 2: `headless_chrome` 1.0.22 (CDP, sync)
- Prós: manutenção ativa, API alta com helpers (`wait_for_element`, screenshots, PDF).
- Contras: modelo síncrono/bloqueante conflita com tokio; emulação/interceptação menos expostas; menos controle para popups e frames.

### Opção 3: WebDriver (`thirtyfour` 0.37.x ou `fantoccini` 0.22.x)
- Prós: maduros; `mobileEmulation` via capabilities do ChromeDriver.
- Contras: processo extra (ChromeDriver); controle limitado de rede/storage/tracing; sessão HTTP por comando; semântica de auto-wait própria; pior encaixe para stealth/emulação fina.

### Opção 4: crate `playwright` (port Rust)
- Prós: API mais próxima do original.
- Contras: **abandonada** — última release 0.0.20 em 2022, incompatível com Playwright atual. Rejeitada.

### Opção 5: sidecar Playwright/Node
- Prós: paridade imediata.
- Contras: mantém Node + `node_modules` em produção, IPC e ciclo de vida duplos; contraria o objetivo da migração. Rejeitada como produção; **aceita apenas como oráculo de testes** (`SidecarPlaywrightDriver` atrás de feature flag).

## Decision

Usar **`chromiumoxide` (CDP)** como implementação de produção, **sempre atrás do trait `BrowserDriver`** definido em `docs/02-arquitetura.md`. O oráculo Node permanece durante as fases de paridade via feature flag.

## Rationale

- Atende tokio, tem manutenção ativa e dá acesso ao CDP completo — necessário para emulação mobile, init scripts, bloqueio de recursos, storage state e tracing.
- O trait isola o risco: se a fidelidade de emulação exigir trocar o crate, os fluxos não mudam.
- WebDriver e sidecar não atendem ao requisito de produção sem Node; o port abandonado é inviável.

## Consequences

### Positive
- Controle total sobre launch args (herda a lista de `browser.js`), emulação, rede e diagnósticos.
- Núcleo testável com `MockDriver` (sem browser) e oráculo Node disponível.
- Sem runtime Node e sem `node_modules` no artefato final.

### Negative
- Implementar do zero: auto-wait por visibilidade, `:has-text`, `$$eval` eficiente, `waitForEvent('page')`, `storageState` (cookies + localStorage), presets de device, `recordVideo`.
- Mais código de infraestrutura de browser (~2–3k linhas) e testes correspondentes.

### Risks
- **Fidelidade de emulação**: validar cedo com testes que inspecionam `navigator.userAgent`, `navigator.webdriver`, viewport/touch, headers — comparando com o oráculo.
- **Vídeo**: `Page.startScreencast` é trabalhoso; entra na fase de diagnósticos, com fallback para trace+screenshot.

## Implementation Notes

- Copiar/portar a cascata de launch e os args de `reference/ali-coins/browser.js:39-188`.
- Emulação: `Emulation.setUserAgentOverride`, `setDeviceMetricsOverride`, `setTouchEmulationEnabled` com os valores do preset Pixel 7.
- Init script equivalente: `Page.addScriptToEvaluateOnNewDocument` (esconder `navigator.webdriver`, criar `window.chrome.runtime`).
- Rede: `Fetch.enable` para bloquear `image|media|font` + telemetria, preservando a semântica do oráculo.
- Cookie de auth a validar: `xman_us_t` e `login_aliyunid_ticket`.

## Related Decisions

- ADR-0002: estrutura do workspace onde o driver vive (`ali-coins-browser`).
- ADR-0006: uso do Node como oráculo de paridade.
