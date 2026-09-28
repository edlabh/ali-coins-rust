# Manual de Execução em Nuvem (VPS)

Guia para rodar o port Rust em um servidor (Oracle Cloud, AWS, VPS genérica) com cron.

## 1. Sessão: exportar local, importar na nuvem

Provedores de nuvem têm IPs de datacenter que o AliExpress trata com risco elevado (captcha/2FA no login). O fluxo recomendado é delegar a sessão:

```bash
# No seu computador (conexão residencial), após um login bem-sucedido:
./target/release/ali-coins export-session            # grava session_token.txt (0600)
./target/release/ali-coins export-session --all      # todas as contas

# No servidor:
./target/release/ali-coins import-session < session_token.txt
./target/release/ali-coins import-session --all      # varre session_token*.txt e remove após importar
```

- O token é cifrado (`v3`) e autocontido; o import faz auto-roteamento por conta.
- `--keep-tokens` (ou `KEEP_SESSION_TOKENS=true`) preserva os arquivos de token.
- A sessão importada fica em `session.json.enc` (0600) e é renovada a cada execução.

## 2. Agendamento no cron

Exemplo: **08:30 America/Sao_Paulo**. Converta para o fuso da VM (se a VM está em UTC, 08:30 BRT = **11:30 UTC**):

```cron
# >>> ali-coins-rust (port) >>>
# 08:30 America/Sao_Paulo = 11:30 UTC (VM em Etc/UTC).
CRON_TZ=America/Sao_Paulo
30 8 * * * /home/ubuntu/ali-coins-rust/run_all.sh --json >> /home/ubuntu/ali-coins-rust/cron.log 2>&1
# <<< ali-coins-rust (port) <<<
```

- Use **marcadores** e `crontab -l | sed` para editar sem tocar nas demais linhas.
- Nem todo cron suporta `CRON_TZ`; alternativa: agende no horário local equivalente e documente.
- Para evitar sobreposição com outro bot, deixe uma janela (ex.: Node às 06:30 BRT, Rust às 08:30 BRT).
- Rotação de `cron.log` é feita pelo `run_all.sh` (5 MB → `cron.log.1`).

## 3. Modo Docker (experimental)

> ⚠️ **Experimental — prefira o binário direto.**
> O modo Docker ainda é experimental. Em VPS pequenas (1 vCPU / ~1 GB), compilar a
> imagem compatível pode levar **horas** e esgotar RAM/swap; o caminho recomendado é
> **compilar no seu computador e instalar o binário** no servidor (seções 1–2), usando
> o Docker apenas se precisar de isolamento. Valide o modo Docker antes de confiar o
> cron a ele (o marcador `.docker-ready` existe justamente para essa troca consciente).

`tools/deploy/docker-run-vps.sh` roda o binário numa imagem com Chromium e limites de recursos:

```bash
./docker-run.sh checkin --json
```

- Mesmos limites do Node: `--memory=768m --memory-swap=1536m --pids-limit=256 --shm-size=256m`.
- `--user $(id -u):$(id -g)` + `/etc/passwd` e `/etc/group` montados para ler os arquivos 0600.
- `ALI_COINS_CHROME` aponta para o Chromium do Playwright (cache do host montado em `/pw:ro`).
- Use o marcador `.docker-ready` no diretório para trocar de execução nativa para Docker com segurança.

## 4. VPS com pouca RAM (512 MB–1 GB)

- Adicione swap (2–4 GB): `sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile`.
- Mantenha o Chromium em modo low-memory (padrão) e o container limitado a 768 MB.
- Páginas pesadas podem levar ~35 s para o `domcontentloaded`; ajuste `NAV_TIMEOUT=90000` se necessário.
- Evite compilar no servidor sem limite de jobs: prefira compilar em outra máquina e
  copiar o binário; se compilar localmente, use `CARGO_BUILD_JOBS=1` e swap extra (o
  build completo pode levar horas e ser morto por OOM em hosts de 1 GB).

## 5. Heartbeat (dead man's switch)

Configure `HEARTBEAT_URL` (Healthchecks.io, Uptime Kuma…) para ser avisado se o cron não rodar.
O envio é best-effort (`/start`, sucesso, `/fail`) e nunca altera o exit code.

## 6. Monitoramento e notificações

- Telegram: veja `TELEGRAM.md`. Toda execução (sucesso, já coletado ou falha) notifica.
- `NOTIFY_HOST_LABEL` identifica o host nas mensagens (ex.: `vm-ali-rust`).
