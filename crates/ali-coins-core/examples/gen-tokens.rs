//! Gera tokens com a implementação Rust para o verificador Node
//! (`tools/parity/verify-rust-tokens.sh`). O plaintext é comparado por
//! deep-equal no oráculo, então a ordem das chaves não importa.

use ali_coins_core::crypto::{EncryptOptions, EncryptVersion, encrypt_session};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Serialize)]
struct Fixture {
    generator: &'static str,
    secret: &'static str,
    plaintext: String,
    tokens: BTreeMap<&'static str, String>,
}

fn main() {
    let secret = "parity-test-secret-0123456789abcdef";
    let plaintext = serde_json::to_string_pretty(&json!({
        "cookies": [{ "name": "xman_us_t", "value": "fake-cookie", "domain": ".aliexpress.com" }],
        "origins": []
    }))
    .expect("serializa plaintext");

    let mut tokens = BTreeMap::new();
    tokens.insert(
        "v2",
        encrypt_session(
            &plaintext,
            secret,
            &EncryptOptions {
                version: Some(EncryptVersion::V2),
                ..EncryptOptions::default()
            },
        )
        .expect("v2"),
    );
    tokens.insert(
        "v3",
        encrypt_session(
            &plaintext,
            secret,
            &EncryptOptions {
                n: Some(16_384),
                r: Some(8),
                p: Some(1),
                ..EncryptOptions::default()
            },
        )
        .expect("v3"),
    );
    tokens.insert(
        "v3default",
        encrypt_session(&plaintext, secret, &EncryptOptions::default()).expect("v3default"),
    );

    let fixture = Fixture {
        generator: "cargo run -p ali-coins-core --example gen-tokens",
        secret,
        plaintext,
        tokens,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&fixture).expect("serializa fixture")
    );
}
