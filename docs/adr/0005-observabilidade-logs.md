# ADR-0005: Logs, sanitização e encerramento

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

O oráculo usa `pino` com redaction (`fast-redact` + scrub recursivo + sanitização de query strings) e `pino-pretty` em TTY. Contratos: com `--json`, **stdout é JSON puro e logs vão para stderr**; no crash, heartbeat `fail` + Telegram `failure` + flush e exit 6; no shutdown, flush de webhooks com teto e exit 130/143 em sinais (C-01, C-03, C-07).

## Decision Drivers

- Não vazar segredos em logs (password, token, cookie, query sensível).
- `--json` consumível por monitores (stdout imaculado).
- Encerramento com códigos de saída corretos mesmo em crash/panic.

## Considered Options

### Opção 1: `tracing` + `tracing-subscriber` JSON + redaction própria — **escolhida**
- Prós: padrão do ecossistema; JSON estruturado; camadas; integração com spans.
- Contras: formato de log diferente do pino (documentar como divergência aceitável); redaction precisa ser implementada.

### Opção 2: `slog`/`log` + `serde_json` manual
- Prós: controle total do formato.
- Contras: sem spans/contexto; reinventa subscriber.

### Opção 3: replicar pino byte-a-byte
- Prós: nenhuma divergência de formato.
- Contras: custo alto, frágil, sem valor de contrato (logs não são consumidos por máquina no projeto; apenas `--json` de relatório é).

## Decision

Usar `tracing` + `tracing-subscriber`:
- `--json` → **todas** as linhas de log em stderr em JSON (`{"level":..,"time":..,"msg":..,...}`); stdout reservado ao relatório JSON.
- Sem `--json` e com TTY → formato humano legível (aproximação do `pino-pretty`, **não** byte-a-byte); sem TTY → JSON.
- Camada de redaction própria: lista de chaves (password/passwd/token/cookie/secret/auth/authorization/bearer/credential/api_key/private_key/access_key/senha/pwd/pass), sanitização de query strings e fragments (`token,code,password,session,...`), mascaramento de token de bot (`bot\d+:...`) e de `Bearer`.
- Panic hook: logar `fatal`, acionar heartbeat `fail` + Telegram `failure` best-effort, flush com teto de 5 s e exit 6 (equivalente ao `crash.js`).
- Sinais: `SIGINT`/`SIGTERM` → cancelar execução, fechar browser com teto de 10 s, liberar lock, flush, exit 1 (durante execução) ou 130/143 (durante atraso inicial/PID 1), como o oráculo.

## Rationale

O contrato observável de logs é a **separação stdout/stderr** e a **não exposição de segredos** — não o formato do pino. Replicar pino-pretty consumiria esforço sem cliente que dependa dele.

## Consequences

### Positive
- Segredos continuam redigidos por construção; `--json` permanece parseável.
- Encerramento testável (exit codes, flush, lock liberado).

### Negative
- Formato de log humano difere do original (documentar no CHANGELOG/README do port).
- Chaves de redaction precisam ser mantidas manualmente; risco de esquecer novas.

### Risks
- **Flush de pipes**: Rust não garante flush síncrono de stdout/stderr em `process::exit`; adquirir handles e flush explícito antes de sair (equivalente ao `libs/exit.js`).
- `tracing` com múltiplas threads pode intercalar; usar writer com lock e `MakeWriter`.

## Implementation Notes

- Eventos-chave a manter (C-16): `[Login] Usuário:`, `[Versão]`, cabeçalhos de seção, `[OBSERVABLE-SELECTORS]`, `start_delay`, `account_delay`.
- Webhooks em voo: `JoinSet` + flush com deadline de 5 s (equivalente `flushWebhooks`).
- Níveis: mapear `LOG_LEVEL` (`trace…silent`; `silent` desliga o subscriber).

## Related Decisions

- ADR-0003 (config `LOG_LEVEL`/`--json`), ADR-0004 (segredos em arquivo).
