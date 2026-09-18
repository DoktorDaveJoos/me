use std::{env, path::PathBuf, process::Command};
fn main() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src/macos.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-module-cache-path"])
        .arg(out.join("swift-cache"))
        .arg(source)
        .arg("-o")
        .arg(out.join("me-document-extract"))
        .status()
        .expect("The macOS build needs the Xcode command line tools (swiftc).");
    assert!(
        status.success(),
        "Could not compile the local PDF/OCR helper"
    );
}
