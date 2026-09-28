//! Decoder base64 leniente compatível com `Buffer.from(x, 'base64')` do Node.
//!
//! O Node ignora caracteres fora do alfabeto (inclusive whitespace), aceita
//! o alfabeto URL-safe (`-`/`_`) e **descarta os bits residuais** de um grupo
//! final incompleto — diferente de decoders estritos. Usado apenas na leitura;
//! a escrita usa o encoder padrão do crate `base64` (com padding).

/// Decodifica `input` com a semântica leniente do Node.
///
/// Nunca falha: caracteres inválidos são ignorados e grupos incompletos
/// contribuem apenas com os bytes completos possíveis (bits extras descartados).
pub fn decode_lenient(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;

    for byte in input.bytes() {
        let Some(value) = alphabet_value(byte) else {
            continue;
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((accumulator >> bits) & 0xFF) as u8);
        }
    }

    out
}

/// Valor de um caractere no alfabeto base64/base64url.
fn alphabet_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodifica_canonico() {
        assert_eq!(decode_lenient("aGVsbG8="), b"hello");
        assert_eq!(decode_lenient("AAECAwQ="), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn ignora_invalidos_e_whitespace() {
        assert_eq!(decode_lenient("aG Vs\nbG8="), b"hello");
        assert_eq!(decode_lenient("!!!!"), b"");
        assert_eq!(decode_lenient(""), b"");
    }

    #[test]
    fn bits_residuais_sao_descartados_como_no_node() {
        // 2 chars = 12 bits -> 1 byte (bits residuais ignorados).
        assert_eq!(decode_lenient("YQ"), b"a");
        // 1 char = 6 bits -> nenhum byte.
        assert_eq!(decode_lenient("Y"), b"");
        // 3 chars = 18 bits -> 2 bytes.
        assert_eq!(decode_lenient("YWJ"), b"ab");
    }

    #[test]
    fn alfabeto_url_safe() {
        assert_eq!(decode_lenient("-_8"), decode_lenient("+/8"));
    }
}
