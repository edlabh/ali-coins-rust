# Manual de Release

Guia de versionamento, publicação e rollback do `ali-coins-rust`.

## Versionamento

- **SemVer**: `MAJOR.MINOR.PATCH` em `Cargo.toml` (versão do workspace);
- mudanças relevantes para o usuário entram no [`CHANGELOG.md`](../../CHANGELOG.md)
  em *Keep a Changelog*;
- a versão também aparece no rodapé das notificações (`v<versão>`).

## Fluxo de release (a partir da `main`)

1. Atualize a versão no `Cargo.toml` e o `CHANGELOG.md`;
2. Rode a suíte local:
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
3. Crie e envie a tag:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
4. O workflow [`release.yml`](../../.github/workflows/release.yml) compila os binários
   e publica um tarball + `.sha256` por plataforma:
   - `ali-coins-rust-<versão>-linux-x86_64.tar.gz`
   - `ali-coins-rust-<versão>-macos-aarch64.tar.gz` (Apple Silicon)
   - `ali-coins-rust-<versão>-macos-x86_64.tar.gz` (Intel)
   - `ali-coins-rust-<versão>-windows-x86_64.tar.gz` (`ali-coins.exe`)

### Dry-run sem tag (validar o workflow)

Para ensaiar a compilação multi-OS **sem** criar tag/release:

1. GitHub → **Actions → release → Run workflow** (`workflow_dispatch`);
2. informe a versão usada só nos nomes (padrão: `dry-run`);
3. o workflow compila os 4 alvos e **guarda os tarballs como artefatos** — a
   publicação no release só acontece em push de tag `v*`.

## Release manual (fallback)

Se o workflow falhar, replique localmente:

```bash
VERSION="0.2.0"
cargo build --release -p ali-coins-cli
ARCHIVE="ali-coins-rust-${VERSION}-linux-x86_64.tar.gz"
tar -C target/release -czf "$ARCHIVE" ali-coins
sha256sum "$ARCHIVE" > "$ARCHIVE.sha256"

# Publicar (gh autenticado)
gh release create "v${VERSION}" "$ARCHIVE" "$ARCHIVE.sha256" \
  --title "v${VERSION}" --generate-notes
```

## Checklist para releases maiores (1.x)

- [ ] CI verde em `main` (fmt, clippy, testes, cobertura, SBOM, Docker);
- [ ] paridade verificada nas fixtures (`./tools/parity/generate-fixtures.sh`);
- [ ] smoke real de check-in e tarefas numa conta de teste;
- [ ] migração de sessão/token exercitada (`--migrate`, `--rotate`);
- [ ] README/manuais revisados (instalação, nuvem, Telegram);
- [ ] rollback documentado e testado (binário anterior + sessão compatível);
- [ ] changelog com notas de breaking changes.

## Rollback

```bash
# Binário
install -m 755 ali-coins.bak-<data> ali-coins

# Docker
docker tag ali-coins-rust:anterior ali-coins-rust:latest

# Sessões e tokens permanecem compatíveis entre versões (ADR-0004).
```

## Referências Cruzadas

- [Índice dos manuais](README.md)
- [Instalação Linux](INSTALL_LINUX.md#10-atualização-e-rollback)
- [Observação da Fase 6](../07-fase6-observacao.md)
- [Roadmap](../03-roadmap.md)
