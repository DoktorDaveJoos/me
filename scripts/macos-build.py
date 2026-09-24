#!/usr/bin/env python3
"""Build, verify and atomically install the permanent ME Dev channel."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parent.parent
DEV_ID = "local.me.desktop.dev"
DATA_NAMES = ("vault", "accounts", "device-account.json", "interface.json", "codex-inbox")


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, REPO / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


signer = module("me_signer", "macos-sign-bundle.py")
configuration = module("me_configuration", "macos-bundle-plist.py")


def channel_paths(channel, home, target):
    if channel == "dev":
        return home / "Applications/ME Dev.app", home / "Library/Application Support/ME Dev"
    if channel == "preview":
        return target / "preview/ME Preview.app", target / "preview/data"
    return target / "release/ME.app", None


def channel_plist(template, channel, previous, environment, data, revision, version):
    # Product/channel identity never comes from an environment override.
    value = configuration.configured_plist(template, previous, environment)
    name, identifier = {"dev": ("ME Dev", DEV_ID), "preview": ("ME Preview", "local.me.desktop.preview"),
                        "release": ("ME.", "local.me.desktop")}[channel]
    value.update(CFBundleName=name, CFBundleDisplayName=name, CFBundleIdentifier=identifier,
                 CFBundleVersion=str(version), MERevision=revision)
    launch = value.setdefault("LSEnvironment", {})
    launch["ME_BUILD_CHANNEL"] = channel
    if data is not None:
        launch["ME_VAULT_DIR"] = str(data / "vault")
    value["NSScreenCaptureUsageDescription"] = "ME shows previews of open windows so you can choose one to prepare credentials. Images stay on your Mac."
    return value


def requirement(bundle):
    result = subprocess.run(["/usr/bin/codesign", "-d", "-r-", str(bundle)], capture_output=True, text=True, check=True)
    for line in (result.stdout + result.stderr).splitlines():
        if "designated => " in line:
            return line.split("designated => ", 1)[1]
    raise ValueError("No designated code requirement found")


def verify_identity(bundle, expected):
    actual = requirement(bundle)
    if actual != expected:
        raise ValueError("Signing identity changed. Refusing to replace the installed app and invalidate its permissions.")
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", "-R", "=" + expected, str(bundle)], check=True)


def signature_manifest(bundle):
    return {name: requirement(bundle if name == "app" else bundle / "Contents/MacOS" / name)
            for name in ("app", "me-context", "me-mcp")}


def verify_manifest(bundle, manifest):
    for name, expected in manifest.items():
        verify_identity(bundle if name == "app" else bundle / "Contents/MacOS" / name, expected)


def snapshot(root):
    result = {}
    for name in DATA_NAMES:
        source = root / name
        if source.is_symlink():
            raise ValueError("Migration refuses symbolic links in account data")
        if not source.exists():
            continue
        for path in [source, *sorted(source.rglob("*"))] if source.is_dir() else [source]:
            relative = path.relative_to(root)
            if "codex-inbox" in relative.parts:
                provider_parts = relative.parts[relative.parts.index("codex-inbox") + 1:]
                if provider_parts and provider_parts not in (("auth.json",), ("installation_id",)):
                    continue  # Provider caches, skills and temporary executable links regenerate.
            if path.is_symlink():
                raise ValueError("Migration refuses symbolic links in account data")
            if path.is_file():
                with path.open("rb") as stream:
                    digest = hashlib.sha256()
                    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                        digest.update(chunk)
                result[str(path.relative_to(root))] = digest.hexdigest()
    return result


def migrate_data(source, destination):
    if destination.exists():
        raise ValueError("ME Dev already has a data folder; refusing to overwrite it with legacy data")
    before = snapshot(source)
    if not any(p.endswith("/header.json") for p in before):
        raise ValueError("No legacy vault found; refusing an empty migration")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".me-migration-", dir=destination.parent) as temporary:
        staged = Path(temporary) / "data"
        staged.mkdir(mode=0o700)
        for relative in before:
            old, new = source / relative, staged / relative
            new.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            shutil.copy2(old, new)
        if snapshot(staged) != before or snapshot(source) != before:
            raise ValueError("Account files changed during migration. Quit ME and retry; the source remains untouched")
        os.rename(staged, destination)
    return len(before)


def install(staged, destination, expected):
    signer.assert_stopped(destination)
    if expected:
        verify_manifest(staged, expected)
    backup = destination.with_name("." + destination.name + ".previous")
    if backup.exists():
        shutil.rmtree(backup)
    if destination.exists():
        os.rename(destination, backup)
    try:
        os.rename(staged, destination)
    except BaseException:
        if backup.exists():
            os.rename(backup, destination)
        raise
    # The previous executable is disposable build output, not account data.
    if backup.exists():
        shutil.rmtree(backup)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("channel", choices=["dev", "preview", "release"])
    parser.add_argument("--migrate-from", type=Path, metavar="LEGACY_APP")
    options = parser.parse_args()
    if sys.platform != "darwin":
        raise ValueError("The macOS bundle commands require macOS")
    if options.migrate_from and options.channel != "dev":
        raise ValueError("Only ME Dev can import an existing development installation")
    home = Path.home()
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target")).resolve()
    destination, data = channel_paths(options.channel, home, target)
    state_dir = home / "Library/Application Support/ME Development"
    state_path = state_dir / "channel.json"
    saved = json.loads(state_path.read_text()) if options.channel == "dev" and state_path.exists() else {}
    if saved and (saved.get("bundle_id") != DEV_ID or saved.get("path") != str(destination)):
        raise ValueError("The saved development channel identity/path does not match. Refusing to change it")
    signer.assert_stopped(destination)
    previous_path = destination / "Contents/Info.plist"
    previous = plistlib.loads(previous_path.read_bytes()) if previous_path.exists() else {}
    if options.migrate_from:
        signer.assert_stopped(options.migrate_from.resolve())
        if saved or destination.exists():
            raise ValueError("ME Dev is already installed; migration is a one-time setup")
        previous = plistlib.loads((options.migrate_from / "Contents/Info.plist").read_bytes())
        if previous.get("CFBundleIdentifier") != "local.me.registration-test":
            raise ValueError("Expected the legacy ME Registration Test app")
    if saved and not os.environ.get("ME_CODESIGN_IDENTITY"):
        os.environ["ME_CODESIGN_IDENTITY"] = saved["certificate"]
    identity = signer.signing_identity(destination)
    if saved and identity != saved["certificate"]:
        raise ValueError("ME Dev's certificate is pinned. Refusing an environment override")
    if identity == "-" and options.channel != "preview":
        raise ValueError("ME Dev and release bundles require a certificate; ad-hoc signing is limited to previews")
    expected = saved.get("requirements") or (signature_manifest(destination) if destination.exists() else None)
    # Validate the existing app against the pinned identity before doing any work.
    if saved and destination.exists():
        verify_manifest(destination, expected)
    args = [str(REPO / "scripts/cargo"), "build", "--locked", "-p", "me-app", "-p", "me-agent"]
    if options.channel != "dev":
        args.append("--release")
    if options.channel != "release":
        args.extend(["--features", "me-app/development-tools"])
    subprocess.run(args, cwd=REPO, check=True)
    build = target / ("debug" if options.channel == "dev" else "release")
    revision = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], cwd=REPO, text=True).strip()
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO):
        revision += "+dirty"
    value = channel_plist(plistlib.loads((REPO / "apps/desktop/packaging/macos/Info.plist").read_bytes()),
                          options.channel, previous, os.environ, data, revision, int(time.time()))
    if options.channel == "dev" and "ME_ACCOUNT_API_URL" not in value["LSEnvironment"]:
        raise ValueError("Set ME_ACCOUNT_API_URL once to configure the development account service")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".me-build-", dir=destination.parent) as temporary:
        staged = Path(temporary) / destination.name
        macos = staged / "Contents/MacOS"
        macos.mkdir(parents=True)
        for name in ("me", "me-mcp"):
            shutil.copy2(build / name, macos / name)
        subprocess.run(["xcrun", "swiftc", "-O", str(REPO / "apps/desktop/platform/macos/credential-context.swift"),
                        "-o", str(macos / "me-context")], check=True)
        (staged / "Contents/Info.plist").write_bytes(plistlib.dumps(value))
        licenses = staged / "Contents/Resources/Licenses"
        shutil.copytree(REPO / "apps/desktop/assets/licenses", licenses)
        shutil.copy2(REPO / "apps/desktop/assets/fonts/OFL.txt", licenses / "Geist-OFL.txt")
        signer.sign_bundle(staged, identity)
        manifest = signature_manifest(staged)
        if expected:
            verify_manifest(staged, expected)
        if options.migrate_from:
            signer.assert_stopped(options.migrate_from.resolve())
            source = Path(previous["LSEnvironment"]["ME_VAULT_DIR"]).parent
            count = migrate_data(source, data)
            print(f"Preserved and verified {count} account files; the legacy copy is untouched.")
        install(staged, destination, expected)
    if options.channel == "dev":
        state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        state = {"bundle_id": DEV_ID, "path": str(destination), "certificate": identity, "requirements": manifest}
        with tempfile.NamedTemporaryFile(mode="w", dir=state_dir, delete=False) as stream:
            json.dump(state, stream, indent=2)
            temporary_state = stream.name
        os.replace(temporary_state, state_path)
    print(f"Installed {destination.name} ({revision}) at {destination}")
    print("Launch this installed app from Finder. Build updates preserve its certificate-backed permission identity.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
