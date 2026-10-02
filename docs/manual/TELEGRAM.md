# Manual do Telegram

Guia de configuração das notificações via bot do Telegram: criação do bot,
`CHAT_ID`, variáveis, testes, eventos notificados e resolução de problemas.

## Recursos das Notificações

- Resumo diário do check-in e das tarefas (saldo, moedas ganhas, streak, duração);
- Alertas dedicados: streak quebrado, 2FA, captcha, lock ativo, falha;
- **Mensagem consolidada multi-conta** com todas as contas em sequência;
- Aviso de **sessão importada expirada** com ação recomendada;
- Envio com retry em 5xx/429, fallback de formatação e truncamento seguro (4096).

## Passo 1: Criar o Bot no Telegram com o @BotFather

1. Abra o Telegram e converse com [@BotFather](https://t.me/BotFather);
2. Envie `/newbot`, escolha nome e usuário do bot;
3. Copie o **token** (`123456789:AA...`) — é o `TELEGRAM_BOT_TOKEN`.

## Passo 2: Inicializar a Conversa com o Bot

Envie qualquer mensagem (`/start`) para o seu bot. **Sem isso**, a API recusa o
envio (`403: bot can't initiate conversation with a user`).

## Passo 3: Obter o `CHAT_ID`

### Método A: @userinfobot (mais rápido)

1. Converse com [@userinfobot](https://t.me/userinfobot);
2. Ele responde com o seu `Id` (ex.: `123456789`).

### Método B: nativo via API do Telegram

```bash
curl -s "https://api.telegram.org/bot<TELEGRAM_BOT_TOKEN>/getUpdates" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['result'][-1]['message']['chat']['id'])"
```

> Para **grupos/canais**, adicione o bot ao grupo e use o ID negativo
> (ex.: `-1001234567890`).

## Passo 4: Configurar o `credentials.env`

```bash
# Ativar o envio de notificações
TELEGRAM_ENABLED=true

# Token fornecido pelo @BotFather (formato: números:caracteres)
TELEGRAM_BOT_TOKEN="123456789:AA..."

# Seu ID de usuário ou ID do grupo/canal
TELEGRAM_CHAT_ID="123456789"

# Chat IDs específicos para contas secundárias (opcional):
# TELEGRAM_CHAT_ID_2="111222333"
# TELEGRAM_CHAT_ID_3="444555666"

# Enviar também a notificação INDIVIDUAL de cada conta, além do consolidado.
# Com false (padrão), contas que usam o mesmo chat do consolidado têm a mensagem
# individual suprimida (evita rajada); o consolidado é sempre enviado.
# TELEGRAM_PER_ACCOUNT=false

# Notificação silenciosa (sem som)
# TELEGRAM_SILENT=false

# Timeout em milissegundos para a API do Telegram (padrão: 15000)
# TELEGRAM_TIMEOUT_MS=15000

# Identificação do host nas mensagens (opcional; útil para Docker/VPS)
# NOTIFY_HOST_LABEL="vm-ali-rust"
```

> Webhook genérico (Discord/Slack/HTTP) usa `NOTIFY_WEBHOOK_URL` — veja o
> [README](../../README.md#notificações).

## Passo 5: Como Testar a Integração

```bash
# 1. Teste dedicado (envia manual_test e imprime o resultado)
ali-coins notify-test

# 2. Validação com notificação de dry-run
ali-coins --dry-run --notify --json
```

Saída esperada do `notify-test`: **"Mensagem de teste enviada pelo Telegram."**
Se falhar, veja a [seção 7](#7-resolução-de-problemas-troubleshooting).

## Eventos Notificados e Formatação

| Evento | Quando ocorre |
|---|---|
| `dry_run` | `--dry-run --notify` |
| `manual_test` | `ali-coins notify-test` |
| `success` / `already_collected` | fim do `all`/`checkin`/`tasks` |
| `failure` | falha de execução |
| `lock_active` | exit 3 (outra instância) |
| `streak_break` | exit 4 (sequência quebrada) |
| `2fa_required` | exit 5 (2FA sem terminal) |
| `captcha_required` / `captcha_cooldown_released` | desafio anti-bot e fim da pausa |
| `multi_account_report` | execução `--all` |

### Exemplo: sucesso unificado

```text
🔔 AliExpress Moedas - Notificação
Status: success
👤 Conta: ed***@gmail.com
🪙 Ganhas hoje: +47 moedas (check-in +1 / tarefas +46)
📅 Sequência: 5 dias
💰 Saldo: 1165 moedas
⏱️ Duração: 16m 36s
🖥️ Host: vm-ali-rust (v0.1.0)
```

### Exemplo: já coletado

```text
ℹ️ AliExpress Moedas - Já Coletado
👤 Conta: ed***@gmail.com
📅 Sequência: 5 dias
💰 Saldo: 1160 moedas
```

### Exemplo: falha

```text
🔴 ali-coins — 02/10/2026 08:30:00
⚠️ Erro: Request timed out
👤 Conta: ed***@gmail.com
🖥️ Host: vm-ali-rust (v0.1.0)
```

### Aviso de Sessão Remota Expirada

Quando a falha está ligada a uma sessão importada que expirou, a mensagem de
falha ganha o bloco:

```text
⚠️ Aviso de Sessão Remota:
A sessão em uso foi importada de outro host (via import_session.js) e parece ter
expirado ou sido invalidada pelo AliExpress.
💡 Ação necessária: É necessário gerar uma nova sessão executando node export_session.js
no servidor de origem e importá-la neste host com node import_session.js.
```

No port, o fluxo é `ali-coins export-session` / `ali-coins import-session`
([CLOUD_SESSIONS.md](CLOUD_SESSIONS.md#3-delegação-de-sessão-gerar-local--exportar-para-nuvem)).

### Exemplo: multi-conta

```text
✅ AliExpress Moedas - Multi-Conta (Sucesso) — 02/10/2026
[1] ed***@gmail.com: 08:30 → 08:42 (próxima: 08:45)
[2] ag***@gmail.com: 08:45 → 08:58
⏱️ Duração Total: 28m 10s
📅 Data: 02/10/2026
🖥️ Host: vm-ali-rust (v0.1.0)
```

## Parâmetros de Linha de Comando (CLI)

| Flag | Efeito |
|---|---|
| `--notify` / `--no-notify` | Força ligar/desligar a notificação (último vence) |
| `--heartbeat` / `--no-heartbeat` | Força ligar/desligar o heartbeat |
| `--json` | Relatório JSON também no stdout (a notificação continua) |

## Agendamento no Cron

```cron
30 11 * * * /home/ubuntu/ali-coins-rust/run_all.sh --json >> /home/ubuntu/ali-coins-rust/cron.log 2>&1
```

O `run_all.sh` rotaciona o `cron.log` ao passar de 5 MB; para rotação por tempo,
veja [CLOUD_SESSIONS.md#10](CLOUD_SESSIONS.md#10-higiene-e-rotação-de-logs-logrotate).

## 7. Resolução de Problemas (Troubleshooting)

### 1. `HTTP 401: Unauthorized`

Token inválido ou revogado. Gere outro com o @BotFather e atualize
`TELEGRAM_BOT_TOKEN` (reinicie o cron/execução).

### 2. `HTTP 403: Forbidden: bot was blocked by the user`

O usuário bloqueou o bot. Desbloqueie no Telegram (ou use outro chat).

### 3. `HTTP 400: Bad Request: chat not found`

`TELEGRAM_CHAT_ID` errado ou a conversa com o bot nunca foi iniciada
([Passo 2](#passo-2-inicializar-a-conversa-com-o-bot)).

### 4. `HTTP 403: Forbidden: bot can't initiate conversation with a user`

Abra a conversa com o bot e envie `/start` — depois repita o `notify-test`.

### 5. Timeout / erro de rede

O port tenta 3× com backoff (800 ms · 2ⁿ, teto 2,5 s, jitter) só em 5xx/429;
**timeout ambíguo não é repetido** (evita mensagem duplicada). Aumente
`TELEGRAM_TIMEOUT_MS` se a rede for lenta.

### 6. `⚠️ Aviso de Sessão Remota` recebido

A sessão importada expirou: gere uma nova no host de origem
(`ali-coins export-session`) e importe (`ali-coins import-session`).

## Códigos de Saída e Eventos Notificados

| Exit | Evento |
|---|---|
| `0`/`2` | `success` / `already_collected` |
| `1` | `failure` (ou `captcha_required`) |
| `3` | `lock_active` |
| `4` | `streak_break` |
| `5` | `2fa_required` |
| `6` | `failure` (crash) |

## Como Desativar

```bash
TELEGRAM_ENABLED=false
```

Ou pontualmente na linha de comando: `ali-coins all --no-notify`
(o heartbeat continua, a menos que use `--no-heartbeat`).

## Referências Cruzadas

- [Índice dos manuais](README.md)
- [Instalação Linux](INSTALL_LINUX.md)
- [Sessões e nuvem](CLOUD_SESSIONS.md)
- [Releases](RELEASING.md)
