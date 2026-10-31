//! Test vectors for the phone link, shared with the Android app: fixed keys run through `snow`
//! (the engine's Noise) give exact handshake and transport bytes, and the app's own Noise code must
//! produce the same bytes. The committed file is checked here against `snow`; to make a fresh one
//! (new random keys), run this test with `THRALL_UPDATE_VECTORS=1`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState};
use thrall_server::remote::seal;

const PROLOGUE: &[u8] = b"thrall-remote-v1";
const UPDATE_ENV: &str = "THRALL_UPDATE_VECTORS";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Vector {
    pattern: String,
    prologue: String,
    /// Pre-shared key (hex) at position 1, for the pairing pattern.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    psk: String,
    phone_static: String,
    phone_static_public: String,
    phone_ephemeral: String,
    engine_static: String,
    engine_static_public: String,
    engine_ephemeral: String,
    /// Plain payloads (hex) and the messages that carry them.
    message1_payload: String,
    message1: String,
    message2_payload: String,
    message2: String,
    /// The handshake hash both sides end with.
    handshake_hash: String,
    /// A frame from the phone, sealed into link pieces (marker byte + text), each a Noise message.
    phone_frame: String,
    phone_pieces: Vec<String>,
    /// A frame from the engine, big enough to need several pieces.
    engine_frame_len: usize,
    engine_frame_seed: u8,
    /// Its pieces, base64.
    engine_pieces: Vec<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Vectors {
    note: String,
    vectors: Vec<Vector>,
}

fn file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../android/app/src/test/resources/noise_vectors.json")
}

/// The engine's big frame: bytes `seed, seed+1, …` wrapping, as text-safe ASCII letters.
fn engine_frame(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| b'a' + ((i + seed as usize) % 26) as u8)
        .collect()
}

struct Keys {
    phone: (Vec<u8>, Vec<u8>),
    phone_e: Vec<u8>,
    engine: (Vec<u8>, Vec<u8>),
    engine_e: Vec<u8>,
}

fn run(pattern: &str, psk: Option<&[u8; 32]>, keys: &Keys, m1: &[u8], m2: &[u8]) -> Vector {
    let params: snow::params::NoiseParams = pattern.parse().unwrap();
    let mut init = Builder::new(params.clone())
        .local_private_key(&keys.phone.0)
        .unwrap()
        .remote_public_key(&keys.engine.1)
        .unwrap()
        .prologue(PROLOGUE)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&keys.phone_e);
    let mut resp = Builder::new(params)
        .local_private_key(&keys.engine.0)
        .unwrap()
        .prologue(PROLOGUE)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&keys.engine_e);
    if let Some(psk) = psk {
        init = init.psk(1, psk).unwrap();
        resp = resp.psk(1, psk).unwrap();
    }
    let mut init: HandshakeState = init.build_initiator().unwrap();
    let mut resp: HandshakeState = resp.build_responder().unwrap();
    let mut buf = vec![0u8; 65_535];
    let mut out = vec![0u8; 65_535];
    let n = init.write_message(m1, &mut buf).unwrap();
    let message1 = buf[..n].to_vec();
    let got = resp.read_message(&message1, &mut out).unwrap();
    assert_eq!(&out[..got], m1);
    let n = resp.write_message(m2, &mut buf).unwrap();
    let message2 = buf[..n].to_vec();
    let got = init.read_message(&message2, &mut out).unwrap();
    assert_eq!(&out[..got], m2);
    let hash = init.get_handshake_hash().to_vec();
    assert_eq!(hash, resp.get_handshake_hash());
    let mut phone = init.into_transport_mode().unwrap();
    let mut engine = resp.into_transport_mode().unwrap();
    let phone_frame = br#"{"v":1,"id":"h","type":"hello","protocols":[1],"client":{"kind":"android","name":"Thrall","version":"0"},"token":""}"#;
    let phone_pieces = seal(&mut phone, phone_frame).unwrap();
    // Just over one piece, so the second piece is short.
    let (len, seed) = (65_600, 7);
    let engine_pieces = seal(&mut engine, &engine_frame(len, seed)).unwrap();
    Vector {
        pattern: pattern.to_owned(),
        prologue: hex(PROLOGUE),
        psk: psk.map(|p| hex(p)).unwrap_or_default(),
        phone_static: hex(&keys.phone.0),
        phone_static_public: hex(&keys.phone.1),
        phone_ephemeral: hex(&keys.phone_e),
        engine_static: hex(&keys.engine.0),
        engine_static_public: hex(&keys.engine.1),
        engine_ephemeral: hex(&keys.engine_e),
        message1_payload: hex(m1),
        message1: hex(&message1),
        message2_payload: hex(m2),
        message2: hex(&message2),
        handshake_hash: hex(&hash),
        phone_frame: hex(phone_frame),
        phone_pieces: phone_pieces.iter().map(|p| hex(p)).collect(),
        engine_frame_len: len,
        engine_frame_seed: seed,
        engine_pieces: engine_pieces.iter().map(|p| STANDARD.encode(p)).collect(),
    }
}

fn keys_from(v: &Vector) -> Keys {
    Keys {
        phone: (unhex(&v.phone_static), unhex(&v.phone_static_public)),
        phone_e: unhex(&v.phone_ephemeral),
        engine: (unhex(&v.engine_static), unhex(&v.engine_static_public)),
        engine_e: unhex(&v.engine_ephemeral),
    }
}

fn fresh_keys() -> Keys {
    let b = Builder::new("Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap());
    let pair = || {
        let k = b.generate_keypair().unwrap();
        (k.private, k.public)
    };
    let (phone, engine) = (pair(), pair());
    Keys {
        phone,
        phone_e: pair().0,
        engine,
        engine_e: pair().0,
    }
}

fn generate() -> Vectors {
    let pair_keys = fresh_keys();
    let connect_keys = fresh_keys();
    let mut psk = [0u8; 32];
    getrandom::fill(&mut psk).unwrap();
    Vectors {
        note:
            "Generated by crates/thrall-server/tests/noise_vectors.rs (THRALL_UPDATE_VECTORS=1). \
               Do not edit."
                .to_owned(),
        vectors: vec![
            run(
                "Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s",
                Some(&psk),
                &pair_keys,
                br#"{"name":"Nothing Phone (4a) Pro"}"#,
                br#"{"phone":"ph_test"}"#,
            ),
            run(
                "Noise_IK_25519_ChaChaPoly_BLAKE2s",
                None,
                &connect_keys,
                b"",
                b"",
            ),
        ],
    }
}

#[test]
fn the_shared_vectors_match_the_engines_noise() {
    let path = file();
    if std::env::var_os(UPDATE_ENV).is_some() {
        let text = serde_json::to_string_pretty(&generate()).unwrap() + "\n";
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing: run with {UPDATE_ENV}=1", path.display()));
    let committed: Vectors = serde_json::from_str(&text).unwrap();
    for v in &committed.vectors {
        let psk: Option<[u8; 32]> = (!v.psk.is_empty()).then(|| unhex(&v.psk).try_into().unwrap());
        let again = run(
            &v.pattern,
            psk.as_ref(),
            &keys_from(v),
            &unhex(&v.message1_payload),
            &unhex(&v.message2_payload),
        );
        assert_eq!(&again, v, "{} no longer matches snow", v.pattern);
    }
}
