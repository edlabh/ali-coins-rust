# ADR-0003: Compatibilidade de CLI e configuração

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

O contrato público do projeto inclui: 16 flags (algumas repetíveis/negáveis), ~60 variáveis de ambiente com **coerções diferentes por variável**, mensagens de erro PT-BR (testadas), `credentials.env` com permissão 0600, `accounts.json` com `passwordEnv`/`passwordFile`, dedupe case-insensitive de contas e hash `sha256(user)[0:8]` para paths. Scrapers de cron, scripts `run_*` e usuários finais dependem desse contrato (C-01…C-04).

## Decision Drivers

- Paridade de CLI/env é o contrato mais fácil de quebrar e o mais barato de testar.
- Evitar dependência de shell e de comportamento implícito do Node (`process.argv`, `||`, trim).
- Mensagens de erro observadas por usuários e testes.

## Considered Options

### Opção 1: `clap` derive puro (flags + env)
- Prós: idiomático.
- Contras: `clap` não reproduz as coerções/prioridades por variável nem o comportamento de `--no-notify`/"último vence" do commander; menagens diferentes.

### Opção 2: Parser próprio para tudo (flags e env)
- Prós: controle total.
- Contras: reimplementar parsing de flags, help e version sem ganho.

### Opção 3: `clap` para flags + parser próprio de env — **escolhida**
- `clap` com os nomes/flags exatos; resolução de precedência replicada (`--no-notify` vence `--notify`; `--force`, `--dry-run` detectados de forma direta).
- `Config::from_env()` com **tabela declarativa de variáveis** (nome, tipo, default, coercão, validação) — a tabela vira o teste de paridade.
- `dotenvy` para ler `credentials.env` + chmod 0600; divergências de parsing vs. `dotenv` cobertas por testes com o oráculo.

## Decision

Adotar a Opção 3: `clap` para flags, parser declarativo próprio para env, `dotenvy` para o arquivo, mensagens PT-BR preservadas e subcomandos para os entrypoints.

## Rationale

- Reproduz o que é observável (flags, defaults, mensagens) com o menor código de infraestrutura.
- A tabela declarativa de env é diretamente testável contra o `config.js` do oráculo (um teste por variável e por caso de borda).

## Consequences

### Positive
- Contrato de CLI/env verificável por testes de tabela e execução lado a lado.
- `--json`, `--dry-run`, `--force`, `--no-delay` e precedências implementados sem gambiarras.

### Negative
- Duas fontes de parsing (clap + tabela) exigem teste de integração completo do contrato C-02/C-04.

### Risks
- Divergência sutil em `dotenv` (aspas, `export`, comentários, multilinha): mitificar com corpus de arquivos `.env` reais + comparação com o Node.
- `allowUnknownOption(true)` do commander: decidir tolerância — implementar aviso e continuar para flags desconhecidas (equivalente), com teste.

## Implementation Notes

- Coerções a replicar 1:1 (ver `docs/01-avaliacao.md` §4/R6): booleanos com três famílias (`false/0/off/no` vs `true|1` vs `true/1`/`false/0`), inteiros positivos com fallback silencioso, não-negativos aceitando 0, `LOG_LEVEL` enum com erro, `superRefine` de MIN/MAX e teto 24h, `CAPTCHA_COOLDOWN_HOURS=12`.
- `maskUser`/`maskChatId` portados com testes de snapshot.
- `accounts.json`: `passwordEnv` só de `env::var` explícito (sem fallback de arquivo), `passwordFile` confinado ao diretório com realpath anti-symlink.
- Ordem de precedência: `accounts.json` → `ALI_USER/ALI_PASSWORD` → `ALI_USER_2..20`; dedupe case-insensitive preservando o primeiro.
- `--dry-run` roda antes de qualquer import de browser (equivalente: antes de inicializar `browser`), valida config/contas/URLs e nunca abre Chromium nem adquire lock.

## Related Decisions

- ADR-0002 (workspace), ADR-0006 (testes de paridade).
