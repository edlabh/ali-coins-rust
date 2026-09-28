//! Paridade do perfil mobile com o Playwright instalado no oráculo.
//!
//! Fixture: `tools/parity/fixtures/common/device_pixel7.json`

use ali_coins_browser::launch::{DeviceProfile, pixel7_profile};
use std::path::Path;

#[test]
fn perfil_pixel7_igual_ao_playwright() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/parity/fixtures/common/device_pixel7.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture {} ausente ({err}); rode ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    let expected: DeviceProfile = serde_json::from_str(&raw).expect("fixture válida");
    assert_eq!(pixel7_profile(), expected);
}
