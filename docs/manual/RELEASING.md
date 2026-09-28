# Manual de Release

## Versionamento

- Semver `MAJOR.MINOR.PATCH` no `[workspace.package]` do `Cargo.toml`.
- `CHANGELOG.md` no formato Keep a Changelog (adicione a seção da versão antes do release).

## Publicar

```bash
# 1. Atualize a versão no Cargo.toml e o CHANGELOG.md
# 2. Rode a suíte local
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. Crie a tag e envie
git tag v0.2.0
git push origin v0.2.0
```

O workflow `release.yml` reage a tags `v*`:
1. compila `ali-coins` em release;
2. empacota `ali-coins-rust-<versão>-linux-x86_64.tar.gz` + `.sha256`;
3. publica (ou atualiza) o GitHub Release com notas geradas.

## Artefato manual

```bash
cargo build --release -p ali-coins-cli
tar -C target/release -czf ali-coins-rust-$(date +%F)-linux-x86_64.tar.gz ali-coins
sha256sum ali-coins-rust-*.tar.gz > ali-coins-rust-*.tar.gz.sha256
```

## Rollback

- Reinstale o binário anterior na VPS (o diretório guarda o último instalado) e reinicie o cron;
- Sessões/arquivos são retrocompatíveis (formato `v3` estável);
- O projeto Node original permanece instalado como fallback durante a migração.
