# Manual de Execução em Nuvem e Sessões

Guia para rodar o `ali-coins-rust` em VPS (Oracle Cloud, AWS, GCP, DigitalOcean…)
sem cair em captcha, com sessão delegada, cron confiável e monitoramento.

## Sumário

1. [O desafio de IPs de datacenter (anti-bot)](#1-o-desafio-de-ips-de-datacenter-anti-bot)
2. [Por que o login funciona no PC e falha na nuvem?](#2-por-que-o-login-funciona-no-pc-e-falha-na-nuvem)
3. [Delegação de sessão (gerar local → exportar para nuvem)](#3-delegação-de-sessão-gerar-local--exportar-para-nuvem)
4. [Métodos para transferir a sessão](#4-métodos-para-transferir-a-sessão)
5. [Duração e renovação da sessão](#5-duração-e-renovação-da-sessão)
6. [Troca de contas na nuvem](#6-troca-de-contas-na-nuvem)
7. [Otimizações para VPS com pouca memória](#7-otimizações-para-vps-com-pouca-memória-512-mb---1-gb-ram)
8. [Agendamento no cron](#8-agendamento-no-cron)
9. [Heartbeat (dead man's switch)](#9-heartbeat-dead-mans-switch)
10. [Higiene e rotação de logs (logrotate)](#10-higiene-e-rotação-de-logs-logrotate)
11. [Códigos de saída e alertas](#11-códigos-de-saída-e-alertas)
12. [Referências cruzadas](#12-referências-cruzadas)

## 1. O Desafio de IPs de Datacenter (Anti-Bot)

O AliExpress aplica desafios anti-bot (captcha, slider, 2FA) com mais frequência
para IPs de datacenter. O sintoma típico é: **o login funciona na sua rede
residencial, mas na VPS cai em captcha** — mesmo com usuário/senha corretos.

O port implementa o mesmo tratamento do oráculo:

- pausa de novas tentativas de login por `CAPTCHA_COOLDOWN_HOURS` (12 h);
- `exit 1` + notificação `captcha_required` quando o captcha é detectado;
- `exit 5` + notificação `2fa_required` quando o 2FA aparece sem terminal.

## 2. Por que o Login Funciona no PC e Falha na Nuvem?

Três fatores somados:

1. **Reputação do IP** — o Alibaba pontua datacenter vs. residencial;
2. **Fingerprint do navegador** — o port já emula Pixel 7 (UA, viewport, touch);
3. **Sessão prévia** — uma sessão válida gerada no PC “pula” o login na VPS.

Por isso a estratégia recomendada é **delegar a sessão**: autenticar uma vez na
rede residencial e transferir o token cifrado para a VPS.

## 3. Delegação de Sessão (Gerar Local → Exportar para Nuvem)

### A. Gerar a sessão no computador pessoal

```bash
ali-coins checkin --json          # faz login e grava session.json.enc + meta
```

### B. Exportar o token cifrado

```bash
# v3 (padrão): cifrado com SESSION_SECRET; modo 0600
ali-coins export-session > session_token.txt

# Exibir em tela só quando necessário (evita vazamento em logs)
ali-coins export-session --show-token

# Multi-conta
ali-coins export-session --all --json > todos_os_tokens.json
```

### C. Importar na VPS

```bash
# stdin (recomendado; evita gravar o token em disco na VPS)
ali-coins import-session < session_token.txt

# a partir de arquivo
ali-coins import-session --from-file session_token.txt

# opções úteis
ali-coins import-session --plaintext        # sem criptografia at-rest
ali-coins import-session --keep-tokens      # não remove o token após importar
ali-coins import-session --migrate          # converte session.json legado em .enc
```

O token exportado é vinculado a `SESSION_SECRET` — as duas pontas precisam usar o
mesmo segredo. Se girar a chave na origem, use `--rotate` e depois
`--new-secret-from-env=SESSION_SECRET_NEW`.

## 4. Métodos para Transferir a Sessão

### Método 1: utilitários do próprio port (recomendado)

```bash
# no PC
ali-coins export-session > /tmp/token.txt
# na VPS
ali-coins import-session < /tmp/token.txt
```

### Método 2: SCP direto

```bash
scp session_token.txt usuario@vps:~/ali-coins-rust/
ssh usuario@vps 'cd ~/ali-coins-rust && ali-coins import-session < session_token.txt && rm session_token.txt'
```

### Método 3: here-doc (sem arquivo intermediário)

```bash
ssh usuario@vps 'cd ~/ali-coins-rust && ali-coins import-session' <<'EOF'
<conteúdo do token>
EOF
```

### Método 4: rsync do código/estado sem segredos

```bash
rsync -av --delete \
  --exclude credentials.env --exclude 'session*' --exclude target --exclude .git \
  ali-coins-rust/ usuario@vps:~/ali-coins-rust/
```

> **Nunca** sincronize `credentials.env` em repositório ou canal público; o
> `.gitignore` já bloqueia sessões e segredos.

## 5. Duração e Renovação da Sessão

- Cookies de autenticação costumam durar dias; o port valida a sessão a cada
  execução e **atualiza** os cookies quando a página renova o token.
- Sinais de expiração: `login` no meio da execução, `exit 4/5`,
  mensagens `Sessão inválida` ou o alerta **"Aviso de Sessão Remota"** no Telegram.
- Renovação: repita o fluxo da [seção 3](#3-delegação-de-sessão-gerar-local--exportar-para-nuvem)
  (login local → export → import).
- Backups automáticos de sessão respeitam `SESSION_BACKUP_RETENTION_DAYS`
  (padrão 7) e são podados em cada gravação.

## 6. Troca de Contas na Nuvem

```bash
# Selecionar por índice (1-based) ou usuário
ali-coins all --account 2
ali-coins all --account outra_conta@exemplo.com

# Rodar todas em sequência (multi-conta)
ali-coins all --all
```

Cada conta tem sessão, lock e meta próprios (`session_<hash>.json`,
`session_meta_<hash>.json`). Pausas entre contas:

```bash
ACCOUNT_DELAY_MIN_MS=60000     # 1 min
ACCOUNT_DELAY_MAX_MS=180000    # até 3 min (anti-detecção)
ACCOUNT_BACKOFF_BASE_MS=30000  # backoff exponencial + jitter em falhas
```

A notificação consolidada inclui todas as contas; com `TELEGRAM_PER_ACCOUNT=true`
cada conta também recebe a sua mensagem individual (use `CHAT_ID_<n>` para
direcionar). Detalhes em [TELEGRAM.md](TELEGRAM.md#eventos-notificados-e-formatação).

## 7. Otimizações para VPS com Pouca Memória (512 MB - 1 GB RAM)

- Compile **fora** da VPS ou use a **imagem de runtime** + binário pré-compilado
  ([INSTALL_LINUX.md#7-docker](INSTALL_LINUX.md#7-docker));
- mantenha `CHROMIUM_LOW_MEMORY=true` (padrão) e `ALLOW_MEDIA=false`;
- `CHROMIUM_JS_HEAP_MB=128` se o Chromium morrer por OOM;
- evite `PW_VIDEO`; trace/screenshot só sob demanda (`PW_TRACE=off` é o padrão);
- um container com `--memory=768m --memory-swap=1536m` é suficiente para o fluxo diário;
- limite o Chromium com `--pids-limit=256` (já aplicado no wrapper Docker da VPS).

## 8. Agendamento no Cron

```bash
crontab -e
```

```cron
# 08:30 America/Sao_Paulo (a VPS costuma usar UTC: 11:30)
30 11 * * * /home/ubuntu/ali-coins-rust/run_all.sh --json >> /home/ubuntu/ali-coins-rust/cron.log 2>&1
```

### O clássico "cron não encontra o binário ou variáveis"

- o cron usa um `PATH` mínimo: use **caminhos absolutos** no `run_all.sh` (o
  wrapper já resolve o diretório dele);
- variáveis de ambiente do shell **não** chegam ao cron: coloque tudo em
  `credentials.env` (o port carrega do diretório de trabalho);
- `docker` no cron precisa de permissão: valide com `crontab -l` e logs em
  `/var/log/syslog` (`grep CRON /var/log/syslog`);
- um `cd` errado no script quebra o caminho da sessão: o wrapper faz `cd` para o
  próprio diretório.

### Retentativa

`wrappers/run_all.sh` repete **uma vez** (`--no-delay`) quando o exit é `1`,
espelhando o comportamento do oráculo. Exit `4`/`5` **não** são repetidos.

## 9. Heartbeat (Dead Man's Switch)

O heartbeat avisa um serviço externo se o cron parou de rodar.

```bash
# credentials.env
HEARTBEAT_ENABLED=true
HEARTBEAT_URL="https://hc-ping.com/<uuid>"     # Healthchecks, Uptime Kuma, etc.
HEARTBEAT_TIMEOUT_MS=10000
```

Ações enviadas pelo port:

| Momento | Ação |
|---|---|
| Início da execução | `start` |
| Sucesso | `success` (com o relatório JSON no corpo) |
| Falha | `fail` (com o erro) |

Validação: rode `ali-coins --dry-run --json` e confira o painel do serviço;
depois rode uma execução real. Serviços locais/privados exigem
`ALLOW_PRIVATE_WEBHOOKS=true`.

## 10. Higiene e Rotação de Logs (logrotate)

```bash
# /etc/logrotate.d/ali-coins-rust
/home/ubuntu/ali-coins-rust/cron.log {
    weekly
    rotate 8
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
}
```

```bash
sudo logrotate -d /etc/logrotate.d/ali-coins-rust   # dry-run
sudo logrotate -f /etc/logrotate.d/ali-coins-rust   # aplica
```

O `run_all.sh` também rotaciona o `cron.log` ao passar de 5 MB. Artefatos de
diagnóstico em `scratch/` são regidos por `DIAGNOSTICS_RETENTION_DAYS`.

## 11. Códigos de Saída e Alertas

| Código | Significado | Notificação |
|---|---|---|
| `0` | Sucesso | `success` |
| `1` | Falha | `failure` (e o wrapper tenta 1×) |
| `2` | Sem ação / já coletado | `already_collected` |
| `3` | Lock ativo | `lock_active` |
| `4` | Streak quebrado | `streak_break` |
| `5` | 2FA não-interativo | `2fa_required` |
| `6` | Crash | `failure` (via panic handler) |

Alertas adicionais: `captcha_required` e `captcha_cooldown_released`
(quando a pausa pós-captcha termina). Em multi-conta, o evento consolidado é
`multi_account_report`.

## 12. Referências Cruzadas

- [Índice dos manuais](README.md)
- [Instalação Linux](INSTALL_LINUX.md)
- [Telegram](TELEGRAM.md)
- [Releases](RELEASING.md)
- [Chat IDs e eventos do Telegram](TELEGRAM.md#eventos-notificados-e-formatação)
- [Solução de problemas do Telegram](TELEGRAM.md#7-resolução-de-problemas-troubleshooting)
