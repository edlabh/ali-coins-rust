# Manual do Telegram

Notificações de execução (sucesso, já coletado, falha) via bot próprio. 100% opcional.

## 1. Criar o bot

1. No Telegram, fale com **@BotFather** → `/newbot` → escolha nome e username.
2. Copie o **token** gerado (`123456789:ABC...`).
3. Abra a conversa com o seu bot e clique em **Iniciar** (`/start`) — bots não podem iniciar conversas por política da API.
4. Fale com **@userinfobot** para obter seu **chat ID** numérico.

## 2. Configurar no `credentials.env`

```env
TELEGRAM_ENABLED=true
TELEGRAM_BOT_TOKEN="123456789:ABCdefGHIjklMNOpqrsTUVwxyz123456"
TELEGRAM_CHAT_ID="987654321"
TELEGRAM_SILENT=false

# Opcionais
# TELEGRAM_TIMEOUT_MS=15000
# TELEGRAM_PER_ACCOUNT=false     # envio individual por conta (além do consolidado)
# TELEGRAM_CHAT_ID_2="111222333" # chat por conta secundária
# NOTIFY_HOST_LABEL=vm-ali-rust  # rótulo do host nas mensagens
```

## 3. Testar

```bash
ali-coins notify-test
# esperado: "Mensagem de teste enviada pelo Telegram."
```

Também é possível validar a configuração sem enviar nada:

```bash
ali-coins --dry-run --json | python3 -c "import json,sys; t=json.load(sys.stdin)['telegram']; print(t)"
```

## 4. Eventos notificados

| Evento | Quando |
|---|---|
| `manual_test` | `notify-test` |
| `success` | check-in coletado e/ou tarefas executadas |
| `already_collected` | check-in já constava como feito hoje (exit 2) |
| `failure` | erro no fluxo (inclui mensagem de erro; falha de 2FA/captcha) |

Formatos por conta: chat individual (se configurado) tem precedência sobre o global;
com `TELEGRAM_PER_ACCOUNT=false` e chats iguais, o individual é suprimido.

## 5. Resolução de problemas

| Sintoma | Causa provável | Ação |
|---|---|---|
| `skipped: true` e nada enviado | `TELEGRAM_ENABLED` falso ou token/chat vazios | revise o `credentials.env` |
| Erro `chat not found` | conversa não iniciada com o bot | envie `/start` no bot |
| Falha de timeout | rede lenta na VPS | aumente `TELEGRAM_TIMEOUT_MS` |
| Nada chega, mas `notify-test` funciona | execução não chegou ao fim (crash) | veja o `cron.log`/stderr; o crash handler notifica quando possível |

Notas: mensagens acima de 4096 unidades são truncadas com segurança; retries ocorrem
apenas em 5xx/429 (timeouts ambíguos **não** são repetidos para evitar duplicidade).
