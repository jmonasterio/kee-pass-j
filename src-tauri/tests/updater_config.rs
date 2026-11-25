//! The updater settings in tauri.conf.json. A public key that does not match the signing key,
//! or an endpoint off this repository's release feed, leaves every installed copy unable to
//! update, and nothing reports it until someone wonders why they are still on an old version.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};

fn conf() -> serde_json::Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses")
}

/// Tauri stores the minisign key and signature files base64-encoded once more.
fn unwrap_base64(text: &str) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).expect("base64");
    String::from_utf8(bytes).expect("utf-8")
}

#[test]
fn public_key_verifies_a_file_signed_with_the_release_key() {
    // fixtures/signed.txt.sig: `tauri signer sign` with the key whose private half is the
    // TAURI_SIGNING_PRIVATE_KEY secret. Re-sign it when the key changes.
    let conf = conf();
    let pubkey = conf["plugins"]["updater"]["pubkey"].as_str().expect("plugins.updater.pubkey");
    let key = PublicKey::decode(&unwrap_base64(pubkey)).expect("minisign public key");
    let sig = Signature::decode(&unwrap_base64(include_str!("fixtures/signed.txt.sig"))).expect("minisign signature");
    key.verify(include_bytes!("fixtures/signed.txt"), &sig, false).expect("signature matches the configured key");
    assert!(key.verify(b"something else", &sig, false).is_err());
}

#[test]
fn endpoint_is_this_repositorys_latest_release_feed() {
    let conf = conf();
    let endpoints = conf["plugins"]["updater"]["endpoints"].as_array().expect("plugins.updater.endpoints");
    // https only: the plugin refuses http unless dangerousInsecureTransportProtocol is set.
    assert_eq!(endpoints, &[serde_json::json!("https://github.com/jmonasterio/kee-pass-j/releases/latest/download/latest.json")]);
    // The updater needs the signed update packages to exist at all.
    assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
}
