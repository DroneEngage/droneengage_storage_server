//! Generates an Ed25519 key pair for Server-to-Server (S2S) authentication
//! (scripts/gen_s2s_keys.js).
//!
//! Usage: gen_s2s_keys <server_id>   (cargo run --bin gen_s2s_keys -- <server_id>)
//!
//! Writes ssl_local/<server_id>_private.pem (PKCS#8, chmod 600: the server
//! that CONNECTS out) and ssl_local/<server_id>_public.pem (SPKI: for the
//! accepting server's s2s_trusted_server_keys), relative to the working
//! directory.

use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::SigningKey;
use std::io::Write;
use std::path::Path;

fn main() {
    let Some(v_id) = std::env::args().nth(1).filter(|a| !a.is_empty()) else {
        eprintln!("Error: server_id parameter is required");
        eprintln!("Usage: gen_s2s_keys <server_id>");
        eprintln!("Example: gen_s2s_keys DE_StorageSrv");
        std::process::exit(1);
    };
    let v_dir = Path::new("ssl_local");
    if let Err(e) = std::fs::create_dir_all(v_dir) {
        eprintln!("Error: cannot create {}: {}", v_dir.display(), e);
        std::process::exit(1);
    }
    let v_private = v_dir.join(format!("{}_private.pem", v_id));
    let v_public = v_dir.join(format!("{}_public.pem", v_id));
    let v_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let v_private_pem = v_key.to_pkcs8_pem(LineEnding::LF).expect("PKCS#8 encoding");
    let v_public_pem = v_key.verifying_key().to_public_key_pem(LineEnding::LF).expect("SPKI encoding");

    let mut v_opts = std::fs::OpenOptions::new();
    v_opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut v_opts, 0o600);
    let v_res = v_opts
        .open(&v_private)
        .and_then(|mut f| f.write_all(v_private_pem.as_bytes()))
        .and_then(|_| std::fs::write(&v_public, v_public_pem.as_bytes()));
    if let Err(e) = v_res {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    let (p, q) = (v_private.display(), v_public.display());
    println!("S2S Ed25519 key pair generated for server '{}':", v_id);
    println!("  private : {}", p);
    println!("  public  : {}", q);
    println!();
    println!("Distribution:");
    println!("  1. Copy {} to the server's ssl_local directory", p);
    println!("  2. Copy {} to the accepting server's ssl_local directory", q);
    println!("  3. Add the public key to the accepting server's s2s_trusted_server_keys config:");
    println!("     \"s2s_trusted_server_keys\": {{ \"{}\": \"./ssl_local/{}_public.pem\" }}", v_id, v_id);
    println!();
    println!("For storage server configuration:");
    println!("  \"s2s_my_private_key\": \"./ssl_local/{}_private.pem\"", v_id);
}
