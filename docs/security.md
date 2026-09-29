# Segurança: token do GitHub fora de comandos e URLs

Este documento registra a varredura feita nos fluxos de push (skill de
publicação, remotes e históricos) e a mudança adotada: **o token do GitHub
nunca é passado em strings de comando, URLs de remote ou variáveis inline**.

## Auditoria (29/09/2026)

| Verificação | Resultado |
|---|---|
| `git remote -v` | `origin` limpo (`https://github.com/edlabh/ali-coins-rust.git`) |
| `git config --list --show-origin` | nenhuma credencial/token configurado |
| `~/.git-credentials`, `~/.netrc` | inexistentes |
| Históricos (`~/.bash_history`, `~/.zsh_history`) | 0 ocorrências de `ghp_` |
| Repositório/histórico git (`grep`, `git log -S ghp_`) | 0 ocorrências |
| VM de produção (histórico, git config, repos) | 0 ocorrências; nenhum repo git |

O risco identificado era **operacional**: pushes feitos com
`https://<token>@github.com/...` na linha de comando expõem o segredo à lista
de processos, histórico do shell, logs de CI e transcrições.

## Prática adotada

1. **Cofre local**: `~/.config/ali-coins/github-token` com permissão `0600`
   (diretório `0700`), fora do repositório e nunca versionado.
2. **Credential helper**: `~/.local/bin/git-credential-github-token` responde ao
   protocolo `git-credential` lendo o arquivo (usuário `x-access-token`).
3. **Configuração do git** (escopo global, só para github.com):

   ```bash
   git config --global credential."https://github.com".helper \
     '!/home/<usuario>/.local/bin/git-credential-github-token'
   ```

4. **Remote sempre limpo** — nada de `https://<token>@github.com/...`.
5. **Revogação/rotação**: tokens expostos em chat/logs devem ser revogados no
   GitHub e substituídos no arquivo protegido (nunca em comandos).
6. **CI**: usa `GITHUB_TOKEN`/secrets do próprio GitHub Actions; nenhum token
   de usuário é necessário no pipeline.

## Instalação rápida

```bash
install -d -m 0700 ~/.config/ali-coins
install -m 0755 tools/security/git-credential-github-token.sh \
  ~/.local/bin/git-credential-github-token
umask 077
printf '%s' '<NOVO_TOKEN>' > ~/.config/ali-coins/github-token
chmod 600 ~/.config/ali-coins/github-token
git config --global credential."https://github.com".helper \
  '!'"$HOME"'/.local/bin/git-credential-github-token'
```

## Verificação (sem imprimir o segredo)

```bash
# 1. Helper responde usuário e tamanho da senha
printf 'protocol=https\nhost=github.com\n\n' | \
  ~/.local/bin/git-credential-github-token get | \
  awk -F= '/^username=/{print $1"="$2} /^password=/{print "password=<"length($2)" chars>"}'

# 2. Git autentica sem token na linha de comando
GIT_TERMINAL_PROMPT=0 git ls-remote origin | head -3

# 3. Nada de token em remotes/config
git remote -v && git config --list --show-origin | grep -c ghp_
```

## Prompt gerado (prompt-engineer) — RISEN + RODES

```text
Papel: Você é um engenheiro de segurança de aplicações responsável por eliminar
credenciais de comandos, URLs e históricos.

Objetivo: Garantir que o token do GitHub NUNCA seja passado em strings de comando,
URLs de remote, variáveis inline ou logs — usando um credential helper que lê o
segredo de um arquivo 0600 fora do repositório.

Instruções:
1. Auditar sem imprimir valores: `git remote -v`,
   `git config --list --show-origin`, `~/.git-credentials`, `~/.netrc`,
   históricos (`~/.bash_history`, `~/.zsh_history`) e o repositório/histórico git
   para `ghp_`/`github_pat_`; reportar apenas caminhos e contagens.
2. Criar `~/.config/ali-coins/` (0700) e `github-token` (0600) com o token.
3. Criar `~/.local/bin/git-credential-github-token` que responde `get` com
   `username=x-access-token` e `password=<conteúdo do arquivo>`; ignorar
   `store`/`erase`.
4. Configurar `git config --global credential."https://github.com".helper
   '!/home/<usuario>/.local/bin/git-credential-github-token'`.
5. Versionar o helper (`tools/security/`) e esta prática (docs), sem o token.
6. Limpar ocorrências do token em históricos locais e garantir `origin` limpo.
7. Validar: (a) helper devolve usuário e tamanho da senha; (b) `git push`
   funciona sem token na linha de comando; (c) CI do GitHub verde.
8. Recomendar revogação do token exposto e rotação pelo arquivo protegido.

Formato de saída: relatório com auditoria (caminhos/contagens), mudanças
aplicadas, comandos de verificação e próximos passos.

Checagem de senso: nenhum comando novo contém o token; pushes futuros funcionam
somente pelo helper e o remote permanece sem credenciais.
```
