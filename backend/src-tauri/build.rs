//! Tauri build script and sidecar build dependency declaration.
//!
//! Cargo rerun hints keep desktop packages synchronized with ct-engine source.
//! The script prepares build metadata only and never launches hardware code.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=../ct-engine/src");
    println!("cargo:rerun-if-changed=../ct-engine/Cargo.toml");

    let profile = env::var("PROFILE").expect("PROFILE is unavailable");
    let workspace = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest directory unavailable"))
        .parent()
        .expect("workspace root unavailable")
        .to_path_buf();
    let repository = workspace.parent().expect("repository root unavailable");
    let executable = if cfg!(windows) { "ct-engine.exe" } else { "ct-engine" };
    let outer_target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(|path| repository.join(path))
        .unwrap_or_else(|| repository.join(".cache/cargo"));
    let engine_target_dir = outer_target_dir.join("engine-package");
    let source = engine_target_dir.join(&profile).join(executable);
    let triple = env::var("TARGET").expect("TARGET is unavailable");
    let sidecars = workspace.join("src-tauri/binaries");
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let sidecar = sidecars.join(format!("ct-engine-{triple}{suffix}"));
    // A debug test or a preparation hook can replace the shared bundle input
    // without changing Rust source. Refresh the embedded identity in that case.
    for input in [&source, &sidecar, &workspace.join("Cargo.toml"), &workspace.join("Cargo.lock")] {
        println!("cargo:rerun-if-changed={}", input.display());
    }

    let mut command = Command::new("cargo");
    command
        .current_dir(repository)
        .env("CARGO_TARGET_DIR", &engine_target_dir)
        .args(["build", "--manifest-path", "backend/Cargo.toml", "-p", "ct-engine"]);
    if profile == "release" {
        command.arg("--release");
    }
    let status = command.status().expect("failed to invoke cargo for ct-engine");
    assert!(status.success(), "ct-engine build failed");

    let output = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is unavailable"));
    let bytes = fs::read(&source).expect("failed to read ct-engine");
    fs::write(output.join("ct-engine.sha256"), format!("{:x}", Sha256::digest(&bytes)))
        .expect("failed to record ct-engine identity");
    // The same bytes are tested, bundled, and launched next to the desktop app.
    fs::create_dir_all(&sidecars).expect("failed to create sidecar directory");
    for destination in [output.join(executable), outer_target_dir.join(&profile).join(executable),
        sidecar] {
        if fs::read(&destination).ok().as_deref() != Some(bytes.as_slice()) {
            fs::copy(&source, &destination).expect("failed to stage ct-engine");
        }
    }
    tauri_build::build();
}
