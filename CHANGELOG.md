# Changelog

Todas as mudanças relevantes deste projeto são documentadas aqui.
O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/)
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [0.1.0] - 2026-09-30

Release inicial do port em Rust do `ali-coins` (Node.js + Playwright), em
paridade com o oráculo nas fases 0–4 do roadmap.

### Adicionado

- **Núcleo**: config (tabela declarativa, mensagens PT-BR), cripto v1/v2/v3
  (interop Node↔Rust), sessão (leitura/gravação cifrada, rotação, migração
  legada, import/export de token), lockfile, logging com redaction, tempo,
  relatórios (`unified_report`/`multi_account_report`), notificações
  (Telegram com templates byte-a-byte, webhooks com SSRF, heartbeat).
- **CLI**: `all`, `checkin`, `tasks`, `export-session`, `import-session`,
  `notify-test`, `--dry-run`, `--json`, `--force`, `--account`, `--no-delay`,
  `--rotate`, `--migrate`, relatórios em texto.
- **Browser**: driver CDP (`chromiumoxide`), perfil Pixel 7, bloqueio de
  recursos, storage state, diagnósticos (screenshot/DOM), todas as chamadas
  CDP com teto de tempo.
- **Check-in**: fluxo mobile + extrato desktop como fonte de verdade
  (saldo/bônus/missões), pré-checagem/reuso, sincronização de saldo, coleta de
  água, `resolveStreakDays`, slider humanizado e seletores `:has-text`.
- **Tarefas**: motor portado de `state/verifier/dispatcher/search/prizeland/
  surprise`, com rodadas, tentativas, segunda passada e status contratuais.
- **Extras de paridade**: `START_DELAY_*` no `all`, atraso/`--dry-run` por
  subcomando, heartbeat `start/success/fail`, alerta e exit 4 de streak
  quebrado, eventos dedicados de 2FA/captcha.
- **Segurança**: credential helper do GitHub (token em arquivo 0600, nunca em
  comandos/URLs), gitleaks no CI.

### Corrigido

- Args do chromiumoxide sem `----flag`; `goto` tolerante a `Page.navigate`
  travado; valores do extrato sempre exibidos na mensagem; travamento por CDP
  pendurado em página lenta (tetos por chamada).

[0.1.0]: https://github.com/edlabh/ali-coins-rust/releases/tag/v0.1.0
