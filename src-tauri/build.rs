//! Tauri build script and sidecar build dependency declaration.
//!
//! Cargo rerun hints keep desktop packages synchronized with ct-engine source.
//! The script prepares build metadata only and never launches hardware code.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../crates/ct-engine/src");
    println!("cargo:rerun-if-changed=../crates/ct-engine/Cargo.toml");

    let profile = env::var("PROFILE").expect("PROFILE is unavailable");
    let workspace = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest directory unavailable"))
        .parent()
        .expect("workspace root unavailable")
        .to_path_buf();
    let executable = if cfg!(windows) { "ct-engine.exe" } else { "ct-engine" };
    let outer_target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    let engine_target_dir = outer_target_dir.join("embedded-engine");
    let source = engine_target_dir.join(&profile).join(executable);

    let mut command = Command::new("cargo");
    command
        .current_dir(&workspace)
        .env("CARGO_TARGET_DIR", &engine_target_dir)
        .args(["build", "-p", "ct-engine"]);
    if profile == "release" {
        command.arg("--release");
    }
    let status = command.status().expect("failed to invoke cargo for ct-engine");
    assert!(status.success(), "ct-engine build failed");

    let output = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is unavailable"))
        .join("ct-engine.bin");
    fs::copy(&source, &output).expect("failed to stage embedded ct-engine");
    tauri_build::build();
}
