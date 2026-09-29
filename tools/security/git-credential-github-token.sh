#!/usr/bin/env bash
# Credential helper do git para HTTPS no GitHub.
#
# Objetivo: o token NUNCA aparece em URLs de remote, linhas de comando ou
# logs — ele fica em um arquivo 0600 fora do repositório e é lido pelo git
# apenas no momento da autenticação.
#
# Instalação (Linux):
#   install -d -m 0700 ~/.config/ali-coins
#   install -m 0755 tools/security/git-credential-github-token.sh \
#     ~/.local/bin/git-credential-github-token
#   umask 077
#   printf '%s' '<NOVO_TOKEN>' > ~/.config/ali-coins/github-token
#   chmod 600 ~/.config/ali-coins/github-token
#   git config --global credential."https://github.com".helper \
#     '!/home/<usuario>/.local/bin/git-credential-github-token'
#
# Verificação (não imprime o valor):
#   printf 'protocol=https\nhost=github.com\n\n' | \
#     ~/.local/bin/git-credential-github-token get | \
#     awk -F= '/^username=/{print $1"="$2} /^password=/{print "password=<"length($2)" chars>"}'
#
# Variável opcional: ALI_COINS_GITHUB_TOKEN_FILE (padrão ~/.config/ali-coins/github-token)
set -euo pipefail

TOKEN_FILE="${ALI_COINS_GITHUB_TOKEN_FILE:-$HOME/.config/ali-coins/github-token}"

case "${1:-}" in
  get)
    [ -f "$TOKEN_FILE" ] || exit 0
    printf 'username=x-access-token\n'
    printf 'password='
    cat "$TOKEN_FILE"
    printf '\n'
    ;;
  store|erase)
    # O arquivo é a única fonte; git nunca grava/limpa credenciais por aqui.
    exit 0
    ;;
esac
