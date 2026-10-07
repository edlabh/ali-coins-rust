# Etapa 3 — Roadmap de migração incremental

Depende de: `docs/01-avaliacao.md`, `docs/02-arquitetura.md`, `docs/adr/` (ADR-0006 = método).

**Estimativa total**: 10–13 semanas de 1 dev Rust sênior (sem contar o período de observação do corte).
**Pré-requisito de ambiente**: `rustup` + toolchain **1.85+** — **atendido**: toolchain pinada em **1.99.0** (06/10), instalada e usada local/CI/VM.

---

## Fase 0 — Preparação (2–3 dias)

**Status: concluída em 2026-09-28.** Toolchain Rust 1.98.1 (stable) instalado; workspace com 4 crates compilando, `fmt` e `clippy` verdes; harness de paridade criado; fixtures de cripto geradas do oráculo (`c05bf07`); dry-run Node validado (exit 0) e divergência esperada vs. stub Rust (exit 1) detectada pelo harness.

**Entregáveis**
- Workspace `ali-coins-rust` com os 4 crates vazios compilando; `rust-toolchain.toml`, `rustfmt.toml`, lints.
- `tools/parity/` esqueleto: gera fixtures do Node (tokens v1/v2/v3, `session.json.enc`, `session_meta.json`, `--dry-run --json`, relatórios de exemplo, matriz de env).
- Documentos: este roadmap + ADRs + avaliação revisados e aceitos.

**Critérios de aceite**
- `cargo build`/`cargo test` verdes com crates vazios; `cargo fmt --check` e `clippy -D warnings` no CI local.
- Harness gera e versiona as fixtures sem credenciais reais.

**Rollback**: não se aplica (nenhum efeito externo).

---

## Fase 1 — Core sem browser (2–3 semanas)

Escopo: `core::config`, `crypto`, `session`, `lock`, `logging`, `time`, `report`, `notify`, `exit`; subcomando `--dry-run` funcional; `export-session`/`import-session` completos.

**Status: concluída (2026-09-28/29).** Núcleo completo: `crypto`/`session`/`lock`/`config`/`logging`/`exit`/`time`/`url_guard`/`report` + builders/`notify` (HTTP seguro, Telegram, heartbeat, webhooks) e CLI (`--dry-run`, `export-session`, `import-session`). Render em texto dos relatórios e flags `--rotate`/`--migrate` das CLIs de sessão **concluídos em 29/09**; snapshot byte-a-byte dos eventos do Telegram (D-02) **concluído em 30/09**.

**Entregáveis**
- Parser de env com tabela declarativa + mensagens PT-BR; `credentials.env`/`accounts.json`; paths/hash por conta.
- Cripto v1/v2/v3 + rotação + escrita atômica 0600 **com testes de interop Node↔Rust**.
- Lockfile com corridas e stale; log com redaction; relatórios `unified_report`/`multi_account_report` (serialização) e Telegram/heartbeat/webhook + SSRF.
- CLI `--dry-run` (texto e `--json`) e `export-session`/`import-session`.

**Critérios de aceite**
- 100% dos casos da matriz de env iguais ao Node (12+ fixtures, incluindo erros).
- Interop cripto: 100% de tokens gerados pelo Node decifrados; 100% dos gerados pelo Rust decifrados pelo Node.
- `--dry-run --json`: saída e exit code idênticos (após normalizar placeholders) em 3 cenários (ok, erro de schema, URL inválida).
- Cobertura de linhas do `core` ≥ 80%; nenhum panic em tokens malformados (fuzz básico).

**Rollback**: nada em produção; usuários seguem no Node.

---

## Fase 2 — Camada de browser (1,5–2 semanas)

**Status: concluída (2026-09-28 → 10-01).** Concluído: políticas de launch, perfil Pixel 7, fronteira `BrowserDriver`/`Browser`/`Page`, `MockDriver` e `CdpDriver` (chromiumoxide: launch, init script stealth, navegação/evaluate/seletores/click/scroll/screenshot, emulação de device). **Smoke real executado em 01/10** (libs do Chromium em `~/.local/chromium-libs` + sessão importada; runs reais de check-in/tarefas concluídos), com smoke automatizado `#[ignore]` para ambientes com browser.

Escopo: `browser::cdp` (launch/cascata de args, sanitização de env, low-memory, emulação Pixel 7, init scripts, bloqueio de recursos, contextos/páginas/popups, storageState, cookies) + `diagnostics` (screenshot/trace; vídeo opcional) + `MockDriver`.

**Entregáveis**
- `CdpDriver` + trait implementado; testes portados de `browser_retry.test.js`.
- Script de smoke: lança Chromium headless e verifica UA, viewport/touch, `navigator.webdriver`, bloqueio de imagem, cookies de auth, screenshot/trace em `scratch/`.

**Critérios de aceite**
- Smoke verde em Linux headless; paridade de UA/viewport/DSF com o preset do oráculo.
- Storage state ida-e-volta (cookies + localStorage filtrado) compatível com arquivo gerado pelo Node.
- Pico de RSS do cenário headless low-memory ≤ baseline Node + 10% (documentar se maior).

**Rollback**: feature flag desliga o driver; core continua utilizável.

---

## Fase 3 — Check-in (1,5–2 semanas)

**Status: concluída com validação real (2026-09-28).** Parsers, login (SPA in-page com botão visível), orquestração do `checkin` e CLI validados contra o AliExpress real: run 1 coletou o check-in (exit 0) e run 2 confirmou `alreadyCollected` (exit 2). Evidências em `docs/06-validacao-real.md`. Pré-checagem/reuso/sync/água, `resolveStreakDays` (D-08), releitura de confirmação de quebra + alerta/exit 4 de streak quebrado (D-08) e slider/seletores `:has-text` (D-07) **concluídos em 29–30/09**.

Escopo: `flows::login`, `balance`, `checkin` + `ui::navigation` (goto/retry/closeModals/slider).

**Entregáveis**
- Fluxo completo de check-in (pré-checagem desktop, mobile, coleta, streak, ledger, guards) com os mesmos exit codes (0/1/2/3/5).
- Porte dos testes `collect_guard`, `balance`, `navigation`, `login_reauth`, `captcha_cooldown`, `non_interactive_2fa`.

**Critérios de aceite**
- 5 execuções reais consecutivas (conta de teste, opt-in) com mesma classificação (já coletado × coletado), saldo e streak do Node no mesmo período.
- 2FA sem TTY aborta em <5 s com exit 5; captcha/cooldown respeitados; streak quebrado classificado igual.
- `--dry-run` não regride (nenhuma mudança no core).

**Rollback**: cron alterna para o Node (binário antigo mantido na VPS).

---

## Fase 4 — Tarefas (2–3 semanas)

**Status: motor portado e validado ao vivo (2026-09-29).** `tasks.rs` (máquina de estados), `tasks_verifier.rs` (gaveta/extração), `tasks_dispatcher.rs` (ações/busca/scroll/Prize Land) e `tasks_surprise.rs` (itens surpresa) são ports de `state.js`/`verifier.js`/`dispatcher.js`/`search.js`/`prizeland.js`/`surprise.js`. Validação na VM: **+46 moedas** creditadas (missões 0→46, saldo 996→1042), 6 tarefas `Concluída` e o surprise falhando igual ao oráculo (`sem progresso após 3 tentativas`). Evidências em `docs/06-validacao-real.md`. Pendências: **D-10** (toques do "Browse surprise items" não avançam a rodada — o oráculo falhou igual no mesmo dia) e minigames/quizzes desativados por `SKIP_APP_ONLY_TASKS` (padrão do oráculo).

Escopo: `flows::tasks` (state, verifier, surprise, search, prizeland, dispatcher) e loop de `tasks`.

**Entregáveis**
- Extração da gaveta, classificação/priorização, surprise (3 cards + fallback de detalhe), busca, minigames, retries/2ª passada, contabilidade via ledger.
- Porte dos testes `tasks`, `timing` (equivalência de payloads).

**Critérios de aceite**
- 5 execuções reais: mesmos status por tipo de tarefa, mesmos ganhos de ledger (± tolerância documentada) e mesmo comportamento de timeout/retry.
- `TASK_MAX_ACTIONS`/`TASK_MAX_ATTEMPTS`/`TASK_RETRY_*`/`SKIP_APP_ONLY_TASKS` com efeitos observáveis iguais.
- Exit codes 2/1 em cenários "sem ações"/"com falhas" iguais ao Node.

**Rollback**: modo híbrido (check-in Rust + tarefas Node) atrás de wrapper.

---

## Fase 5 — Multi-conta + infra (1,5–2 semanas)

**Status: concluída (2026-09-28 → 10-07).** Concluído: `Dockerfile` multi-stage (builder Rust + runtime Debian com libs do Chromium, usuário 10001, healthcheck dry-run), `.dockerignore` com segredos ignorados, subcomandos `all`/`checkin`/`tasks` para o wrapper, **multi-conta sequencial** (por processo filho, com lock por conta, backoff/jitter, relatório e notificação consolidados — validado só com **mocks**, sem contas reais) e **release `v0.1.0`** publicada. **Hardening do CI (01/10)**: `cargo audit` **bloqueante** + **`cargo deny`** (licenças/bans/fontes, `deny.toml`) desde 06/10, **cobertura** `cargo llvm-cov` com **gate progressivo em 79%** (medido em 06/10: **80,02% de linhas** após a injeção de driver/contexto no CLI e testes de fluxo com mock — **meta de 80% atingida**), **SBOM CycloneDX** publicado como artefato, **Dependabot** (cargo/actions/docker) e `Dockerfile` com args de baixa memória (`CARGO_BUILD_JOBS=1`, LTO off) para build na VPS. **Imagem própria no runner da VM concluída em 01/10** (`Dockerfile.runtime` + binário pré-compilado; build de segundos). **Encerramento em 07/10**: **release `v1.0.0` publicada** (4 alvos: Linux x86_64, macOS aarch64/Intel e Windows; dry-run via `workflow_dispatch` validado em 06/10) e **imagem v1.0.0 implantada na VM** no mesmo dia (build 5m07s, dry-run exit 0, backup de rollback `ali-coins-rust:backup-2026-10-01` mantido). Pendente apenas smoke tests automatizados com browser real (CDP/glue de CLI).

Escopo: multi-conta sequencial com atrasos/backoff/notificações por conta; Docker multi-stage (UID/GID 10001); CI/CD; wrappers.

**Entregáveis**
- Execução de N contas por conta (isolamento, `--account`, `--all`), ETA/agenda entre contas.
- `Dockerfile` multi-stage + `docker-compose.yml` equivalentes; healthcheck `<bin> --dry-run --json`; `run*.sh|bat|ps1` adaptados.
- CI: `fmt`, `clippy`, `test`, cobertura (`cargo llvm-cov`) ≥80%, `cargo audit`/`deny`, Gitleaks, smoke Docker, SBOM; Dependabot `cargo`; release por tag (Cargo.toml + CHANGELOG).

**Critérios de aceite**
- Execução de 3 contas com fixtures: isolamento de sessão/lock/notificação idêntico.
- Container: healthcheck ok, exit code propagado, pico de memória/PIDs medido (`docker stats`) ≤ limites do compose.
- Pipeline verde nos 3 SOs (ou alvos acordados) com cache de build.

**Rollback**: container Node continua publicado como fallback.

---

## Fase 6 — Operação paralela permanente e release estável (14 dias de observação)

**Status: iniciada (2026-09-30); observação suspensa.** Decisão de 06/10
([ADR-0007](adr/0007-operacao-paralela-permanente.md)): **os dois projetos
continuam ativos em caráter permanente, em repositórios separados — não há corte
nem desligamento do Node.** O cron do port está **pausado por decisão do
operador** desde 02/10; a janela de 14 dias (para a primeira release estável do
port) começa a contar na reativação, acompanhada em
[`07-fase6-observacao.md`](07-fase6-observacao.md).

**Entregáveis**
- Período paralelo: Node e Rust rodando diariamente (contas distintas) por 14 dias.
- Relatório de paridade final; tag da **primeira release estável do port** (`v1.0.0`).
- Revisão de README/INSTALL/CLOUD_SESSIONS/TELEGRAM e registro da operação
  paralela permanente (ADR-0007).

**Critérios de aceite**
- **Zero divergências** em C-01…C-20 no período (com exceção das flexíveis documentadas).
- Instalação limpa em VM Linux + Docker do zero, seguindo os guias; atualização de uma instalação existente sem perder sessão.
- Procedimento de rollback testado (voltar a imagem Node e reutilizar `session.json.enc`).

**Rollback**: reverter imagem/binário; sessões/formato continuam compatíveis por design (ADR-0004). O Node segue ativo como referência permanente (ADR-0007).

---

## Estratégia de paridade (transversal)

| Contrato | Tipo de teste | Fase |
|---|---|---|
| C-01 exit codes | Matriz de cenários Node×Rust (subprocessos) | 1–5 |
| C-02/C-03 flags e `--json` | Tabela de CLI + diff normalizado de stdout | 1 |
| C-04/C-20 env e credenciais | Tabela de env (12+ fixtures) + corpus `.env` | 1 |
| C-05/C-06/C-07 cripto/sessão/paths | Interop round-trip + fixtures | 1 |
| C-08 lock | Testes de corrida (N processos) + stale/force | 1 |
| C-09/C-10 relatórios | Golden JSON + contabilidade | 1 |
| C-11 datas/streak | Fixtures de extrato multilíngue + LA tz | 1 |
| C-12/C-13/C-14 notify/heartbeat/SSRF | Snapshots + testes de rede mockada + listas IP | 1 |
| C-15 status de tarefas | Tabela de strings contratuais | 4 |
| C-16 logs-chave | Asserções de presença (não byte-a-byte) | 3–4 |
| C-17/C-18 emulação/launch | Smoke real + inspeção no browser | 2 |
| C-19 scratch | Verificação de arquivos/permissões | 2 |

**Normalização**: tempos/durações → `T`, moedas do site → comparadas por faixa, ordem de tarefas → conjunto, timestamps ISO → placeholder.

## Riscos do cronograma

| Risco | Efeito | Mitigação |
|---|---|---|
| Fidelidade do CDP (R2/R3) | Atraso nas fases 2–4 | Spike de 3–5 dias na fase 2 com os helpers críticos antes de prosseguir |
| Variabilidade do site real | Falsos negativos de paridade | Fixtures gravadas + janelas de execução pareadas |
| Scope creep (stealth/anti-bot) | Atraso geral | Congelar paridade 1.7.1; backlog separado |
| Toolchain ausente | Bloqueia fase 0+ | **Resolvido**: `rustup` + toolchain **1.99.0** pinada (06/10), usada em local/CI/VM. |
| Runner `macos-13` retirado | Release Intel quebra/fila infinita | **Corrigido em 06/10/2026**: alvo Intel migrado para `macos-15-intel` (`release.yml`). |
| Manutenção dupla | Custo | Congelamento do upstream + só hotfixes de segurança |

## Questões abertas (aprovação)

1. ~~**Autorizar instalação do Rust toolchain**~~ — **resolvido**: toolchain 1.99.0 pinada e instalada (06/10).
2. **Prioridade de plataformas**: Linux (VPS) primeiro e Windows/macOS em fase posterior — **atendido em 02/10/2026**: macOS e Windows receberam manuais (`docs/manual/INSTALL_MACOS.md`, `INSTALL_WINDOWS.md`), wrappers `.ps1`/`.bat`, descoberta de Chromium por SO e jobs de CI/release multi-plataforma.
3. **Sidecar Playwright**: implementado **apenas como ferramenta de paridade** (`reference/`, ignorado pelo git); não há fallback no produto. Reabrir se quiser fallback opcional.
4. **Vídeo de diagnóstico** (`PW_VIDEO`): implementado como **trace CDP JSON (D-03) + screenshot**; vídeo desligado por padrão (`off`). Reabrir se quiser screencast completo.
5. ~~**Repositório**~~ — **resolvido**: este workspace é o repositório permanente; o clone do oráculo fica em `reference/` (ADR-0007: operação paralela permanente com o Node).
