# Etapa 1 — Avaliação técnica: `edlabh/ali-coins` (Node.js + Playwright)

| Campo | Valor |
|---|---|
| Repositório | https://github.com/edlabh/ali-coins |
| Commit avaliado | `c05bf07` (2026-09-25) — versão **1.7.1** |
| Stack | Node.js ≥ 22 · Playwright 1.63 · commander 15 · zod 4 · pino 10 · undici 8 · dotenv |
| Tamanho | **15.883 linhas** de código JS + **16.941 linhas** de testes (30 arquivos) |
| Clone de referência | `reference/ali-coins/` (este workspace) |
| Alvo | Conversão para Rust preservando contratos observáveis |

> Método: leitura do código no commit acima, com apoio de análise paralela por módulo. Referências no formato `arquivo:linha`.

---

## 1. Inventário funcional

### Entrypoints (CLI)

| Arquivo | Responsabilidade | Entradas | Saídas | Dependências | LOC |
|---|---|---|---|---|---|
| `all.js` | Orquestrador unificado: check-in + tarefas, multi-conta, exit codes 0–6 | CLI, env, `credentials.env`, `accounts.json` | relatório (texto/JSON), notificações, arquivos de sessão, exit code | config, lockfile, session, collect, do_tasks, report, notify, heartbeat, crash | 1088 |
| `collect.js` | Check-in diário + saldo/streak; login/reauth; captcha cooldown | sessão, config | resultado de check-in, metadados de sessão | browser, ui/balance, ui/login, selectors, session | 1031 |
| `do_tasks.js` | Painel "Ganhe mais moedas"; loop de tarefas, retries, 2ª passada | sessão, config | resultados por tarefa, ganho de moedas | browser, libs/tasks/*, ui/diagnostics | 841 |
| `export_session.js` | Exporta sessão cifrada (`--all`, `--account`, `--rotate`, `--show-token`) | sessão em disco | `session_token*.txt` (token v3) | security, session, config | 435 |
| `import_session.js` | Importa por STDIN/arquivo, auto-roteamento por conta, `--migrate` | token v3 (stdin/arquivo ≤2 MiB) | `session.json.enc` + meta | security, session, config | 559 |

### Núcleo

| Arquivo | Responsabilidade | Contratos principais | LOC |
|---|---|---|---|
| `config.js` | Schema Zod de ~60 vars, credenciais, multi-conta, paths, CLI commander | erros PT-BR, coercões por variável, hash de conta | 1158 |
| `security.js` | Cripto de sessão v1/v2/v3 (scrypt + AES-256-GCM), escrita atômica 0600 | formato de token, params scrypt, mensagens de erro | 965 |
| `lockfile.js` | Lock atômico (hostname+PID), stale timeout, refresh, sinais | `LockActiveError`, exit 3, hardlink/rename anti-TOCTOU | 631 |
| `logger.js` | pino + redaction (chaves e query strings), roteamento stdout/stderr | `--json`: logs em stderr, stdout puro | 305 |
| `time_utils.js` | Fuso `America/Los_Angeles` (dia contábil), formatos, backoff, esperas abortáveis | `formatDuration` (`45s`, `1m 20s`), streak, jitter | 208 |

### `libs/`

| Módulo | Responsabilidade | LOC |
|---|---|---|
| `libs/session.js` | Ciclo de vida da sessão, paths por conta, meta, clear/backup, rotação, retenção | 1219 |
| `libs/report.js` | Schema/contabilidade dos relatórios, texto/JSON, webhook (Discord/Telegram genérico) | 1135 |
| `libs/notify.js` | Mensagens Telegram (13 eventos), retries 3x, truncamento 4096, escaping HTML | 1090 |
| `libs/url_guard.js` | Anti-SSRF (IPv4/IPv6, NAT64/6to4/Teredo), pinning de DNS por hop, `safeFetch` | 491 |
| `libs/heartbeat.js` | Dead man's switch (`/start`, base, `/fail`), fallback GET 405 | 386 |
| `libs/tasks/state.js` | Status textual contratual, priorização e classificação de tarefas | 347 |
| `libs/tasks/verifier.js` | Extração da gaveta via 1 `$$eval`, auto-cura de skeleton, revalidação por título | 476 |
| `libs/tasks/surprise.js` | Toca 3 cards, assinaturas de URL/card, popups, `goBack`, fallback de detalhe | 604 |
| `libs/tasks/dispatcher.js` | Roteia tipos de tarefa (surprise/search/prizeland/game/review/navegação) | 212 |
| `libs/tasks/search.js` | Busca com query fixa `fone bluetooth` | 61 |
| `libs/tasks/prizeland.js` | Minigame/água da Fazenda | 35 |
| `libs/storage_filter.js` | Allowlist de localStorage (login/user/account/token/session/auth/...) | 74 |
| `libs/ui/balance.js` | Saldo/streak/ledger bilíngue, datas PT, cache de contexto desktop | 579 |
| `libs/ui/login.js` | Login SPA, slider, 2FA (fail-fast sem TTY), validação de cookie | 196 |
| `libs/ui/navigation.js` | `gotoWithRetry`, `waitAndClick`, `closeModals`, slider humanizado | 226 |
| `libs/ui/diagnostics.js` | Trace/screenshot/vídeo, DOM hash SHA-256, dir `scratch/` 0700 | 267 |
| `libs/selectors.js` | ~90 seletores CSS/`:has-text` com fallbacks em cascata | 95 |
| `libs/webhooks.js` | Tracker de webhooks em voo + flush com teto | 62 |
| `libs/exit.js` / `libs/crash.js` | Flush e shutdown (130/143), crash global → exit 6 | 228 |
| `libs/timing.js` | Cronômetro idempotente por etapa | 87 |

### Infra e testes

- `Dockerfile` (node:22-slim + Playwright shell, `appuser` UID/GID 10001, healthcheck dry-run JSON), `docker-compose.yml` (768M/256 PIDs, shm 256m, cap_drop ALL), `docker-run.example.sh` (mede pico de RSS/PIDs, propaga exit code).
- Scripts `run*.sh|bat|ps1`, `setup_*.sh|bat|ps1`, `generate_secret.*`; testes dedicados verificam ancoragem, UTF-8/BOM, `LSS 22`, `exec`/`@args`.
- CI: matriz Ubuntu/Windows/macOS × Node 22, gate de cobertura **≥80% linhas**, Gitleaks, CodeQL, build Docker + smoke; release por tag com seção do CHANGELOG.

---

## 2. Arquitetura atual

**Fluxo de execução (`all.js`)**

```
crash handler → parse CLI → loadConfig(dry-run?) → loadAccounts → banner/versão
→ start delay (opcional, abortável) → lockfile global → heartbeat start
→ [por conta] loadSession/validate → collect.runCheckin → doTasks.runTasks
   → report (texto + JSON + webhook) → Telegram por conta
→ heartbeat success/fail → release lock → flush → exit code
```

**Acoplamentos e pontos de extensão**

- `libs/ui/*` depende diretamente da API `Page` do Playwright (selectors, `$eval`, `waitForSelector`, `route`, `goBack`); é a fronteira mais cara de substituir.
- `libs/report.js` consome resultados de check-in/tarefas e o meta da sessão; o schema JSON é o contrato mais observável para integradores.
- `libs/notify.js` consome o relatório e o config; mensagens são snapshotadas nos testes (contrato textual).
- A maioria das funções aceita `options` injetáveis (usado pelos testes); `Math.random`, `sleep` e relógio são injetáveis em pontos-chave.
- `libs/selectors.js` é o único repositório de seletores — bom ponto de isolamento.
- `time_utils` fixa `REPORT_TIMEZONE` no load do módulo (efeito global).

**Estado global**: logger singleton; lock com timer de refresh; contexto desktop em cache opcional (`DESKTOP_REUSE_CONTEXT`); `SELECTORS`; cache de compile do Node.

**Concorrência**: contas processadas **sequencialmente**; uma página/"aba" por vez; concorrência real existe no lockfile (processos), no retry com timer e no flush de webhooks em voo. Não há paralelismo intra-conta.

---

## 3. Qualidade e testes

**Garantias**

- 30 arquivos de teste, execução serial, cobertura mínima de 80% de linhas no CI.
- Cobertura forte em: cripto/formato v3 e permissões 0600, sessão/rotação/prune, lockfile (corridas de 6 processos, stale, TOCTOU, sinais), config/Zod (mensagens PT-BR), relatório (contabilidade, teto 32KB, SSRF), notify (templates byte-a-byte, retries), url_guard (listas IPv4/IPv6), selectors (especificidade + parse real no Chromium quando disponível), scripts de instalação.

**Lacunas relevantes**

- Nenhum teste ponta-a-ponta com o **DOM real** do AliExpress: todos usam mocks/fixtures — a migração pode passar nos testes e falhar no site.
- `runCheckin`/`runTasks` integrados com browser real não são exercitados (ordem, tempos, popups, SPA).
- Exit codes raramente verificados via processo real (a maioria chama funções).
- Interop de cripto com implementação externa não é testada (será obrigatória no port).
- Divergências entre mocks e DOM real (ex.: `selectors.smoke` usa matcher por substring, não CSS real).

---

## 4. Riscos

| # | Risco | Sev. | Evidência | Mitigação na migração |
|---|---|---|---|---|
| R1 | Interop cripto v3 (base64 leniente do Node, parser tolerante a campos extras, scrypt OpenSSL) | **Alta** | `security.js:541-605`, `560-570` | Portar parser/formato 1:1; teste cruzado Node⇒Rust e Rust⇒Node; manter mensagem genérica de auth |
| R2 | Emulação mobile/CDP divergente do Playwright (Pixel 7, UA, touch, init scripts) | **Alta** | `browser.js:469-497` | Camada de driver + testes de `navigator.webdriver`, viewport, UA; comparar headers/JS com o oráculo |
| R3 | Semântica Playwright ausente no CDP (`:has-text`, auto-wait, `$$eval`, `waitForEvent('page')`) | **Alta** | `selectors.js`, `verifier.js:136-256` | Implementar helpers equivalentes (polling de visibilidade, matcher de texto, snapshot DOM) e cobrir com testes |
| R4 | SSRF/DNS pinning com `reqwest` (connector custom, Happy Eyeballs, IPs embutidos) | **Alta** | `url_guard.js:100-173,301-478` | `reqwest::dns::Resolve` + redirect manual; portar testes de listas e revalidar por hop |
| R5 | Lockfile cross-platform (`link`, `utimes`, `process.kill(pid,0)`, EPERM) | Média-Alta | `lockfile.js:379-430,449-547` | Usar `std::fs::hard_link`/`OpenOptions`, `nix`/`windows-sys` para sinais; testes de corrida |
| R6 | Coerções env divergentes (Zod, `||`, trim de secret inconsistente) | Média | `config.js:91-199`, `security.js:411-417` | Parser de env próprio + tabela de testes de coerção lado a lado |
| R7 | Redaction/sanitização de logs (pino + fast-redact + hooks) | Média | `logger.js:14-91` | Camada de redaction própria; testes com payloads sensíveis |
| R8 | Contabilidade/streak (fuso LA, ciclo de 7, ledger) | Média | `report.js:139-357` | Testes dourados com fixtures do oráculo; `chrono-tz` |
| R9 | Artefatos de diagnóstico (trace/vídeo/dom hash) via CDP `Tracing` | Média | `diagnostics.js:50-258` | Implementar trace por CDP; vídeo é o item mais custoso (avaliar `Page.startScreencast`) |
| R10 | Docker/CI reproduzindo UID 10001, libs do Chromium, gate 80% | Média | `Dockerfile:59-95`, `ci.yml` | Multi-stage Rust + `cargo llvm-cov`; manter contrato de exit codes |
| R11 | Testes dependentes de mocks não detectam divergência real | Média | `tasks.test.js`, `browser_retry.test.js` | Testes de contrato + execução real controlada (dry-run e smoke) antes do corte |
| R12 | Crescimento de escopo (stealth/anti-bot) no meio da migração | Média | `browser.js`, `navigation.js:171-219` | Congelar paridade 1.7.1; melhorias só após corte |

---

## 5. Complexidade de migração por módulo

Esforço: **P** ≤ 2 dias · **M** ≤ 1 semana · **G** > 1 semana (estimativas preliminares, 1 dev Rust experiente).

| Módulo | Complexidade | Risco | Estratégia | Esforço |
|---|---|---|---|---|
| `security.js` + `libs/session.js` (cripto/persistência) | Média | Alto (R1) | Portar 1:1 + interop Node↔Rust | G |
| `config.js` (schema/coercões) | Alta | Médio (R6) | Parser próprio + tabela de testes | G |
| `lockfile.js` | Alta | Médio (R5) | Portar algoritmo + testes de corrida | M |
| `logger.js` + `libs/exit.js` + `libs/crash.js` | Média | Médio (R7) | tracing + redaction + panic hook | M |
| `time_utils.js` + `libs/timing.js` | Média | Baixo | `chrono-tz`; RNG semeável | P-M |
| `libs/report.js` | Alta | Médio (R8) | Structs serde + testes dourados | G |
| `libs/notify.js` | Alta | Médio | Templates + testes snapshot | M-G |
| `libs/heartbeat.js` | Média | Baixo | reqwest + testes | P |
| `libs/webhooks.js` + cliente em `report.js` | Média | Baixo | JoinSet + timeout | P-M |
| `libs/url_guard.js` | Alta | Alto (R4) | Portar 1:1 + resolver custom | G |
| `browser.js` (launch, emulação, bloqueio) | Alta | Alto (R2) | Driver trait + CDP | G |
| `libs/ui/diagnostics.js` | Média | Médio (R9) | CDP Tracing/Screenshot; vídeo depois | M |
| `libs/selectors.js` | Baixa | Baixo | Dados estáticos | P |
| `libs/ui/navigation.js` | Média | Alto (R3) | Helpers CDP + testes | M-G |
| `libs/ui/balance.js` | Alta | Médio | Portar parsers + fixtures | G |
| `libs/ui/login.js` | Média | Médio | 2FA/fail-fast + slider | M |
| `collect.js` | Média-Alta | Médio | Orquestração + guards | G |
| `libs/tasks/verifier.js` | Alta | Alto (R3) | Snapshot DOM + revalidação | G |
| `libs/tasks/surprise.js` | Alta | Alto | Popups/goBack/fallback | G |
| `do_tasks.js` + `dispatcher.js` + `state.js` | Alta | Médio | Máquina de estados + contratos textuais | G |
| `libs/tasks/search.js` / `prizeland.js` | Baixa | Baixo | Portar direto | P |
| `libs/storage_filter.js` | Baixa | Baixo | Portar direto | P |
| `export_session.js` / `import_session.js` | Média | Médio (R1) | CLI própria + interop | M |
| `all.js` (orquestração/exit codes) | Alta | Médio | Fluxo explícito + testes de exit | G |
| Docker/CI/scripts (infra) | Média | Médio (R10) | Multi-stage + targets | M-G |

---

## 6. O que **não** deve ser portado 1:1

1. **Injeção de dependências via `options`** (padrão dos testes JS) → traits e injeção explícita de relógio/RNG/sleep.
2. **Validação Zod em runtime** → `serde` + validação manual onde os defaults/coerções importam; o schema Zod vira tabela de referência, não código.
3. **Mutação de objetos e estado global** (`time_utils` fixa timezone no load) → `Config` imutável passada explicitamente; timezone como campo.
4. **`process.exit` disperso** → função única de shutdown com flush e `ExitCode` retornado até `main`.
5. **`pino-pretty` byte-a-byte** → manter JSON canônico; formato humano aproximado (documentar divergência).
6. **Lenência do base64 de entrada na *escrita*** → na decifragem tolerar o que o Node gerava; ao cifrar, emitir base64 padrão idêntico ao Node.
7. **Callbacks `onMobilePageKept`/`keepPage`** → ownership explícito do contexto/página entre fases.
8. **`Math.random` global** → `rand` com RNG injetável/semeável para testes determinísticos.
9. **`require` circular/implícito** → módulos Rust explícitos; sem `lazy_static` desnecessário.
10. **Stealth/anti-bot "melhorias"** → paridade com 1.7.1 primeiro; mudanças depois do corte.

---

## 7. Contratos observáveis a preservar

| ID | Contrato | Fonte |
|---|---|---|
| C-01 | Exit codes 0–6 + 130/143 em sinais; `run_all` retenta só exit 1 | `config.js:493-501`, `all.js`, `run_all.sh:25-37` |
| C-02 | Flags CLI (`--dry-run`, `--force`, `--no-delay`, `--json`, `--notify/--no-notify`, `--heartbeat/--no-heartbeat`, `--all`, `--account`, `--show-token`, `--from-file`, `--plaintext`, `--keep-tokens`, `--migrate`, `--rotate`, `--new-secret-from-env`, `--version`, `--help`) | `config.js:440-505` |
| C-03 | `--json`: stdout 100% JSON; logs em stderr | `logger.js:167-182` |
| C-04 | Nomes/semântica das variáveis do `credentials.env` (~60) | `credentials.env.example` |
| C-05 | Formato de token `v3:N:r:p:salt:iv:tag:ct:base64`; v1/v2 legados; campos IV=12, tag=16, salt=16 | `security.js:426-605` |
| C-06 | Arquivos `session.json(.enc)`, `session_meta*.json`, `session_token*.txt`; permissões 0600; ordem sessão→meta | `libs/session.js:541-610` |
| C-07 | Hash de conta `sha256(user)[0:8]`; paths e locks por conta | `config.js:896-920` |
| C-08 | Lock: hostname+PID+lockId, stale 30 min, `--force`, refresh, exit 3 | `lockfile.js:86-96,449-547` |
| C-09 | Schema `unified_report` / `multi_account_report` (campos e tipos) | `report.js:14-122` |
| C-10 | Regras de contabilidade (alreadyCollected=0, ledger, isolamento de moedas de tarefas) | `report.js:263-357` |
| C-11 | Datas: ISO nos campos; exibição `DD/MM/AAAA HH:mm:ss` no `REPORT_TIMEZONE`; dia contábil sempre LA | `time_utils.js:12-66` |
| C-12 | Mensagens Telegram (13 eventos), payloads `sendMessage`, truncamento 4096, escaping HTML | `notify.js:450-853` |
| C-13 | Heartbeat `/start`, base, `/fail`; fallback GET 405; body ≤32KB | `heartbeat.js:101-314` |
| C-14 | Guard SSRF (bloqueio de privados/link-local/metadata), pinning por hop, `ALLOW_PRIVATE_WEBHOOKS` | `url_guard.js:100-478` |
| C-15 | Status textuais de tarefas (`Concluída (3/3)`, `Pendente`, ...) | `state.js:271-332` |
| C-16 | Strings de log-chave (`[OBSERVABLE-SELECTORS]`, cabeçalhos de seção, `[Login] Usuário:`) | `collect.js`, `all.js`, `do_tasks.js` |
| C-17 | Emulação mobile: Pixel 7 (UA/viewport 412×915/DSF ~2.625/touch), `pt-BR`, `navigator.webdriver` nulo | `browser.js:469-497` |
| C-18 | Bloqueio de mídia/telemetria; low-memory flags; `--no-sandbox` condicional | `browser.js:39-188` |
| C-19 | Diretório `scratch/` 0700 e retenção de artefatos (7 dias) | `diagnostics.js:50-62`, `libs/session.js:749-894` |
| C-20 | Arquivo de credenciais `credentials.env` com chmod 0600 e `accounts.json` com `passwordEnv`/`passwordFile` | `config.js:37-40,786-829` |

---

## 8. Custo/benefício e alternativas

| Opção | Prós | Contras | Veredito |
|---|---|---|---|
| **A. Manter Node** | Zero esforço; ecossistema Playwright completo (device presets, trace, vídeo) | Runtime Node + ~256 MB heap + browser; `node_modules`; tipagem fraca onde há contratos críticos | Não atende ao pedido |
| **B. Híbrido (core Rust + sidecar Playwright/Node)** | Paridade de browser imediata; reduz parte do código JS | Mantém Node e `node_modules` em produção; IPC e ciclo de vida duplos; ganho de memória marginal | Útil **só como ponte/oráculo temporário** |
| **C. Migração total para Rust (CDP)** | Binário único; tipagem forte em config/cripto/lock/report; menor RAM fora do browser; CI/release Rust | Curva do CDP sem Playwright; reimplementar helpers e emulação; risco concentrado no browser (R2/R3) | **Recomendado**, em fases, com Node como oráculo |

**Onde está o custo**: núcleo sem browser ≈ 40–50% do esforço; camada de browser + fluxos ≈ 35%; infra/scripts/doc ≈ 15%.
**Onde está o ganho**: o Chromium continuará dominando o RSS (~300–500 MB); o ganho de memória do processo principal é relevante em VPS de 512 MB–1 GB, e o binário único simplifica deploy, start-up e erro handling.

---

## Sense check (Etapa 1)

- [x] Inventário cobre todos os 45 arquivos JS (30 de teste) e artefatos de infra.
- [x] Riscos têm evidência no código e mitigação acionável.
- [x] Contratos observáveis estão enumerados com IDs para os testes de paridade da Etapa 3.
- [x] Nenhuma credencial real foi usada; análise estática + mocks apenas.
- [ ] **Pendente de aprovação**: Etapas 2 (arquitetura/ADRs) e 3 (roadmap) estão em `docs/02-arquitetura.md`, `docs/adr/` e `docs/03-roadmap.md`.
