use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=src");
    let build = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=ME_BUILD_ID={build}");
    println!("cargo:rerun-if-changed=platform/macos/credential-context.swift");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR")).join("me-context");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "platform/macos/credential-context.swift",
            "-o",
        ])
        .arg(&out)
        .status()
        .expect("macOS context helper requires Xcode command-line tools");
    assert!(status.success(), "macOS context helper failed to compile");
    println!("cargo:rustc-env=ME_CONTEXT_HELPER={}", out.display());
}
