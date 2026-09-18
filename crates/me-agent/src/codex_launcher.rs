//! Desktop launches do not inherit a shell's PATH. In particular, an npm
//! `#!/usr/bin/env node` shim can exist while Node is invisible to the app.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub(super) fn find() -> PathBuf {
    if let Some(path) = std::env::var_os("ME_CODEX_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return path;
    }
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
        PathBuf::from("/usr/bin/codex"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for relative in [
            ".local/bin/codex",
            ".bun/bin/codex",
            ".npm-global/bin/codex",
        ] {
            candidates.push(home.join(relative));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("codex")),
        );
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("codex"))
}

fn platform() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(("codex-darwin-arm64", "aarch64-apple-darwin")),
        ("macos", "x86_64") => Some(("codex-darwin-x64", "x86_64-apple-darwin")),
        ("linux", "aarch64") => Some(("codex-linux-arm64", "aarch64-unknown-linux-musl")),
        ("linux", "x86_64") => Some(("codex-linux-x64", "x86_64-unknown-linux-musl")),
        _ => None,
    }
}

pub(super) fn resolve(path: &Path) -> Result<PathBuf, String> {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if resolved.file_name().and_then(|n| n.to_str()) != Some("codex.js") {
        return Ok(resolved);
    }
    let Some(root) = resolved
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "bin"))
        .and_then(Path::parent)
    else {
        return Ok(resolved);
    };
    // Only interpret the official package layout; preserve custom wrappers.
    let manifest = root.join("package.json");
    let official = fs::metadata(&manifest).is_ok_and(|m| m.is_file() && m.len() <= 65536)
        && fs::read(&manifest)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|p| p["name"] == "@openai/codex");
    if !official {
        return Ok(resolved);
    }
    let (package, target) =
        platform().ok_or("This Codex installation doesn't support this platform.")?;
    let mut roots = vec![
        root.join("node_modules/@openai").join(package),
        root.to_owned(),
    ];
    if let Some(scope) = root.parent() {
        roots.push(scope.join(package));
    }
    for package_root in roots {
        for relative in ["bin/codex", "codex/codex"] {
            let candidate = package_root.join("vendor").join(target).join(relative);
            if fs::metadata(&candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            {
                return fs::canonicalize(candidate)
                    .map_err(|_| "Couldn't open the native Codex binary.".into());
            }
        }
    }
    Err("The Codex installation is incomplete. Reinstall or update Codex.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package(temp: &Path) -> (PathBuf, PathBuf) {
        let root = temp.join("node_modules/@openai/codex");
        fs::create_dir_all(root.join("bin")).unwrap();
        let shim = root.join("bin/codex.js");
        fs::write(&shim, "#!/usr/bin/env node\n").unwrap();
        fs::write(root.join("package.json"), r#"{"name":"@openai/codex"}"#).unwrap();
        (root, shim)
    }
    #[test]
    fn npm_shim_and_symlink_resolve_to_installed_native_program() {
        let temp = tempfile::tempdir().unwrap();
        let (root, shim) = package(temp.path());
        let (package, target) = platform().unwrap();
        let native = root
            .join("node_modules/@openai")
            .join(package)
            .join("vendor")
            .join(target)
            .join("bin/codex");
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, "synthetic executable").unwrap();
        fs::set_permissions(&native, fs::Permissions::from_mode(0o700)).unwrap();
        let link = temp.path().join("codex");
        std::os::unix::fs::symlink(&shim, &link).unwrap();
        assert_eq!(resolve(&link).unwrap(), native.canonicalize().unwrap());
        fs::remove_file(&native).unwrap();
        assert!(resolve(&link).unwrap_err().contains("incomplete"));
    }
    #[test]
    fn hoisted_package_and_legacy_vendor_layout_are_supported() {
        let temp = tempfile::tempdir().unwrap();
        let (root, shim) = package(temp.path());
        let (package, target) = platform().unwrap();
        for location in [
            root.parent()
                .unwrap()
                .join(package)
                .join("vendor")
                .join(target)
                .join("bin/codex"),
            root.join("vendor").join(target).join("codex/codex"),
        ] {
            fs::create_dir_all(location.parent().unwrap()).unwrap();
            fs::write(&location, "synthetic").unwrap();
            fs::set_permissions(&location, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(resolve(&shim).unwrap(), location.canonicalize().unwrap());
            fs::remove_file(location).unwrap();
        }
    }
    #[test]
    fn custom_programs_are_not_replaced_by_package_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let (_, shim) = package(temp.path());
        fs::write(
            shim.parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("package.json"),
            r#"{"name":"custom"}"#,
        )
        .unwrap();
        assert_eq!(resolve(&shim).unwrap(), shim.canonicalize().unwrap());
        let native = temp.path().join("codex");
        fs::write(&native, "synthetic").unwrap();
        assert_eq!(resolve(&native).unwrap(), native.canonicalize().unwrap());
    }
}
