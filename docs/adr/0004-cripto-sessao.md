# ADR-0004: Cripto e sessão — interoperabilidade com o token v3 do Node

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

O projeto grava/transporta sessões em token `v3:N:r:p:salt:iv:tag:ct:base64` (scrypt + AES-256-GCM) e mantém compatibilidade com `v1`/`v2`. Usuários reais têm sessões exportadas em máquinas pessoais e importadas em VPS; qualquer divergência de formato quebra o fluxo "export no desktop → import na nuvem" (C-05, R1). O Node usa `Buffer.from(x,'base64')` leniente e `JSON.stringify(...,null,2)` na re-cifragem.

## Decision Drivers

- Interoperabilidade **bidirecional**: Rust lê tokens gerados pelo Node e Node lê tokens gerados pelo Rust.
- Nenhum panic em token malformado: erro → mensagem genérica de autenticação, igual ao oráculo.
- Persistência segura (0600, escrita atômica, sem symlink).

## Considered Options

### Opção 1: `aes-gcm` + `scrypt` (RustCrypto) — **escolhida**
- Prós: puros em Rust, auditáveis, sem OpenSSL; `aes-gcm` com nonce 12 B/tag 16 B; `scrypt::Params::new(log_n, r, p, len, maxmem)` permite replicar N/r/p e o cap de memória.
- Contras: `scrypt` exige `N` potência de 2 — tokens com N inválido que o Node "aceitava sanitizar" precisam ser tratados como falha de auth (nunca panic).

### Opção 2: OpenSSL via `openssl`/`native-tls`
- Prós: mesma biblioteca do Node (paridade de scrypt).
- Contras: dependência de sistema, pior build cross-platform/Docker, histórico de CVEs; desnecessário.

### Opção 3: `ring`
- Prós: ótimo para AES-GCM.
- Contras: sem scrypt, e a API não expõe todos os controles necessários.

## Decision

Usar **`aes-gcm` + `scrypt` + `sha2` + `subtle` + `base64`**, com:
- **Encoder** base64 padrão (`STANDARD`, com padding) — idêntico ao `Buffer.toString('base64')`.
- **Decoder leniente** próprio (ignora caracteres inválidos e aceita padding ausente), usado **apenas na leitura**, espelhando `Buffer.from(x,'base64')`.
- Parser de token tolerante aos formatos v1 (≥4 partes, salt fixo), v2 (≥5 partes), v3 completo (≥8 partes) e v3 compacto (≥5 partes), ignorando o sufixo literal `:base64`.
- Validação de tamanhos antes de derivar a chave (IV=12, tag=16, salt=16); qualquer falha → erro PT-BR genérico de autenticação.
- Escrita segura: temporário `.tmp-<pid>-<hex>` com `mode(0o600)`, fsync de arquivo e diretório, `rename` atômico; fallback com backup para `EBUSY/EXDEV/EACCES`; `O_NOFOLLOW`; limpeza de órfãos >300 s.

## Rationale

`aes-gcm`/`scrypt` são o par canônico do ecossistema Rust, sem dependência de sistema, e cobrem exatamente o envelope. A única divergência (N não potência de 2) é tratada como erro de autenticação — comportamento observável idêntico ao Node (que falha no OpenSSL e cai na mesma mensagem).

## Consequences

### Positive
- Tokens v1/v2/v3 continuam funcionando e a rotação de chave permanece compatível.
- Sem OpenSSL no binário (facilita Alpine/Distroless e cross-compile).

### Negative
- Reimplementar a leniência de base64 e o parser tolerante exige testes dedicados com corpus de tokens "sujos" gerados pelo Node.

### Risks
- **Byte-diff de ciphertext não é possível** (IV aleatório): os testes de interop são de **round-trip** (Node cifra → Rust decifra; Rust cifra → Node decifra) e de formato (regex/contagem de campos).
- Re-cifragem com `JSON.stringify(...,null,2)` muda bytes do payload; a paridade é validada por JSON parseado, não por texto.
- `SCRYPT_N` default depende de RAM total/cgroup (1,5 GiB → 32768; senão 131072): replicar leitura de `/sys/fs/cgroup/memory.max` e `memory.limit_in_bytes`.

## Implementation Notes

- Preservar a inconsistência do oráculo: `assertValidSecret` valida com `trim().length >= 32` mas deriva com a string original; `getEncryptionConfig` usa comprimento sem trim. Documentar em teste.
- Parâmetros legados v1: salt fixo `"ali-coins-session-encryption-v1-scrypt-salt"`, `N=16384, r=8, p=1, maxmem=64MB`.
- `sanitizeScryptParams`: piso 16384, teto `N ≤ 2^20`, `r,p ≤ 16`, cap de memória 256 MB (reduzir N pela metade até caber; fallback `N=16384, r=1`).
- Meta `session_meta*.json`: mesmos campos, mesmos nomes (`savedAt`, `encrypted`, `lastStreakDays`, `lastCaptchaAt`, `lastRotatedAt`, …) e `.passthrough` (Rust: `#[serde(flatten)] extra: Map<String,Value>`).
- Ordem de gravação: sessão → meta; meta sem fsync.
- `--plaintext` apenas desliga o at-rest; `SESSION_SECRET` continua obrigatório para decifrar token.

## Related Decisions

- ADR-0002 (workspace/crates), ADR-0006 (testes de paridade).
