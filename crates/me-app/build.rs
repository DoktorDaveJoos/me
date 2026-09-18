fn main() {
    println!("cargo:rerun-if-changed=src");
    let build = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=ME_BUILD_ID={build}");
}
