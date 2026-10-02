# Manual de Instalação — Windows

Guia para Windows 10 e Windows 11 (x86_64). Para Linux/macOS, veja
[INSTALL_LINUX.md](INSTALL_LINUX.md) e [INSTALL_MACOS.md](INSTALL_MACOS.md).

## Sumário

1. [Requisitos mínimos](#1-requisitos-mínimos)
2. [Método A: binário da release (recomendado)](#2-método-a-binário-da-release-recomendado)
3. [Método B: compilar do zero](#3-método-b-compilar-do-zero)
4. [Configuração de credenciais](#4-configuração-de-credenciais)
5. [Primeira execução](#5-primeira-execução)
6. [Agendamento automático diário (Task Scheduler)](#6-agendamento-automático-diário-task-scheduler)
7. [Exportação e importação de sessão](#7-exportação-e-importação-de-sessão)
8. [Resolução de problemas frequentes no Windows](#8-resolução-de-problemas-frequentes-no-windows)
9. [Referências cruzadas](#9-referências-cruzadas)

## 1. Requisitos mínimos

| Item | Recomendado | Mínimo |
|---|---|---|
| Windows | 11 | 10 (21H2+) |
| RAM | 8 GB | 4 GB |
| Disco | 2 GB livres | 1 GB |
| Chromium | Playwright `chromium-*`, Chrome ou Edge | qualquer ≥ 120 |
| Rust (para compilar) | stable (pinada em `rust-toolchain.toml`) | 1.85+ + Build Tools C++ |

> O port roda **sem Node.js**. O Node só é necessário se você optar por instalar o
> Chromium via `npx playwright install`.

## 2. Método A: binário da release (recomendado)

1. Baixe `ali-coins-rust-<versão>-windows-x86_64.tar.gz` e o `.sha256`;
2. Valide o hash (PowerShell):
   ```powershell
   (Get-FileHash .\ali-coins-rust-<versão>-windows-x86_64.tar.gz -Algorithm SHA256).Hash
   # compare com o conteúdo do .sha256
   ```
3. Extraia (`tar -xzf` funciona no Windows 10+; ou use 7-Zip) e coloque o
   `ali-coins.exe` em uma pasta, por exemplo `C:\ali-coins-rust\`.

O port aceita Chrome ou **Microsoft Edge** (Chromium) já instalados; caso não
encontre, defina `ALI_COINS_CHROME` ou instale o Chromium do Playwright.

## 3. Método B: compilar do zero

### Passo 1: Rust e Build Tools

```powershell
winget install --id Rustlang.Rustup
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

O alvo padrão (`x86_64-pc-windows-msvc`) exige o linker do Visual Studio
(workload **Desktop development with C++**).

### Passo 2: Clonar e compilar

```powershell
git clone <repo> ali-coins-rust
cd ali-coins-rust
cargo build --release -p ali-coins-cli      # target\release\ali-coins.exe
```

### Passo 3: Chromium

```powershell
# Opção 1 (recomendada): Chromium do Playwright
npx playwright install chromium

# Opção 2: Chrome/Edge do sistema
$env:ALI_COINS_CHROME = "C:\Program Files\Google\Chrome\Application\chrome.exe"
```

Sem `ALI_COINS_CHROME`, o port procura o cache do Playwright em
`%USERPROFILE%\AppData\Local\ms-playwright\chromium-*\chrome-win*\chrome.exe`.

## 4. Configuração de Credenciais

```powershell
cd C:\ali-coins-rust
Copy-Item credentials.env.example credentials.env
# Gere o segredo
.\wrappers\generate_secret.ps1        # ou generate_secret.bat
```

Edite o `credentials.env` (`ALI_USER`, `ALI_PASSWORD`, `SESSION_SECRET`). No
Windows, use `notepad credentials.env`. O arquivo deve ficar somente na sua
máquina (nunca versionado).

## 5. Primeira execução

```powershell
.\ali-coins.exe --dry-run --json
.\ali-coins.exe checkin --json
.\ali-coins.exe all --json
.\wrappers\run_all.ps1 --json
```

> Se o PowerShell bloquear scripts:
> `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` (veja o item C do
> [troubleshooting](#c-execução-de-scripts-desabilitada)).

## 6. Agendamento Automático Diário (Task Scheduler)

### Opção 1: via PowerShell (1 comando)

```powershell
schtasks /Create /TN "ali-coins-rust" /SC DAILY /ST 08:30 /F `
  /TR "powershell -ExecutionPolicy Bypass -File C:\ali-coins-rust\wrappers\run_all.ps1 --json"
schtasks /Run /TN "ali-coins-rust"        # testa agora
schtasks /Query /TN "ali-coins-rust" /V /FO LIST
```

### Opção 2: interface gráfica

1. Abra o **Agendador de Tarefas** → *Criar Tarefa*;
2. Disparador: diariamente às 08:30;
3. Ação: iniciar programa
   `powershell.exe` com argumentos
   `-ExecutionPolicy Bypass -File C:\ali-coins-rust\wrappers\run_all.ps1 --json`;
4. Marque **Executar estando o usuário conectado ou não**.

## 7. Exportação e Importação de Sessão

```powershell
# Exportar (PC pessoal, após um login válido)
.\ali-coins.exe export-session > session_token.txt

# Importar (na VPS/máquina de destino)
Get-Content session_token.txt | .\ali-coins.exe import-session
```

Fluxo completo e por que o login em datacenter falha:
[CLOUD_SESSIONS.md](CLOUD_SESSIONS.md#3-delegação-de-sessão-gerar-local--exportar-para-nuvem).

## 8. Resolução de Problemas Frequentes no Windows

### A. `'cargo' não é reconhecido...`

Recarregue o terminal após instalar o Rust (`%USERPROFILE%\.cargo\bin` entra no
`PATH`). Confirme com `cargo --version`.

### B. `VCRUNTIME140.dll` / `MSVCP140.dll` ausente

Instale o **Visual C++ Redistributable 2015-2022 (x64)**:
<https://aka.ms/vs/17/release/vc_redist.x64.exe>.

### C. Execução de scripts desabilitada

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

### D. Alerta do Windows Defender / SmartScreen

O binário não é assinado. Libere a pasta ou o arquivo em *Segurança do Windows →
Proteção contra vírus e ameaças → Exclusões*.

### E. Caminhos longos (`MAX_PATH`)

Use caminhos curtos (ex.: `C:\ali-coins-rust`) ou habilite
`LongPathsEnabled` (Política de Grupo / Registro).

### F. Emojis quebrados nos logs

Use o **Windows Terminal** (recomendado) e, em scripts CMD antigos,
`chcp 65001` para UTF-8. O JSON (`--json`) nunca é afetado.

### G. Baixa memória (1-2 GB ou VMs)

- `CHROMIUM_LOW_MEMORY=true` (padrão) e `ALLOW_MEDIA=false`;
- `CHROMIUM_JS_HEAP_MB=128` se necessário;
- evite execuções paralelas de contas em máquinas pequenas.

### H. Chromium não encontrado

- `npx playwright install chromium`; ou
- `$env:ALI_COINS_CHROME = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"`;
- reinicie o terminal após definir a variável.

## 9. Referências Cruzadas

- [Índice dos manuais](README.md)
- [Instalação Linux](INSTALL_LINUX.md)
- [Instalação macOS](INSTALL_MACOS.md)
- [Execução em nuvem / sessões](CLOUD_SESSIONS.md)
- [Telegram](TELEGRAM.md)
- [Releases](RELEASING.md)
