#!/usr/bin/env python3
"""Keep macOS privacy permissions tied to a certificate, not a build's hash."""
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys


def identities(output):
    return re.findall(r'^\s*\d+\) ([0-9A-Fa-f]{40}) "([^"]+)"', output, re.M)


def choose_identity(available, requested=None, previous_authority=None):
    # Reuse the previous certificate by default. Never silently downgrade a
    # certificate-signed app or switch developer teams when rebuilding.
    if requested == "-":
        if previous_authority:
            raise ValueError("Refusing to replace a certificate-signed app with ad-hoc signing.")
        return "-"
    if requested:
        matches = [item for item in available if requested in item or requested.upper() == item[0].upper()]
    elif previous_authority:
        matches = [item for item in available if item[1] == previous_authority]
    else:
        matches = [item for item in available if item[1].startswith("Developer ID Application:")]
        if not matches:
            matches = [item for item in available if item[1].startswith(("Apple Development:", "Mac Developer:"))]
    if len(matches) != 1:
        raise ValueError("Choose an available certificate with ME_CODESIGN_IDENTITY (certificate name or SHA-1). "
                         "Privacy-enabled builds require a stable signing certificate. "
                         "ME_CODESIGN_IDENTITY=- is only for disposable ad-hoc test builds.")
    return matches[0][0]


def assert_stopped(bundle, processes=None):
    executable = bundle / "Contents/MacOS/me"
    if processes is None:
        processes = subprocess.check_output(["/bin/ps", "-axo", "comm="], text=True)
    if str(executable) in (line.strip() for line in processes.splitlines()):
        raise ValueError(f"Quit {bundle.name} before replacing or signing it, then retry. "
                         "Updating a running bundle invalidates its macOS permission identity.")


def signing_identity(bundle):
    info = subprocess.run(["/usr/bin/codesign", "-d", "--verbose=2", str(bundle)], capture_output=True, text=True)
    authority = re.search(r"^Authority=(.+)$", info.stderr, re.M)
    available = subprocess.check_output(["/usr/bin/security", "find-identity", "-v", "-p", "codesigning"], text=True)
    return choose_identity(identities(available), os.environ.get("ME_CODESIGN_IDENTITY"),
                           authority.group(1) if authority else None)


def sign_bundle(bundle, identity):
    assert_stopped(bundle)
    with (bundle / "Contents/Info.plist").open("rb") as source:
        bundle_id = plistlib.load(source)["CFBundleIdentifier"]
    if not re.fullmatch(r"[A-Za-z0-9.-]+", bundle_id):
        raise ValueError("Invalid bundle identifier")
    # Sign nested executables explicitly, inside out, with stable identifiers.
    # Keep codesign's certificate/Apple anchor requirements; no identifier-only DR.
    for executable, suffix in [("me-context", ".context"), ("me-mcp", ".mcp")]:
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", identity, "--timestamp=none",
                        "--identifier", bundle_id + suffix, str(bundle / "Contents/MacOS" / executable)], check=True)
    subprocess.run(["/usr/bin/codesign", "--force", "--sign", identity, "--timestamp=none", str(bundle)], check=True)
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    if identity == "-":
        print("Ad-hoc test build: Accessibility and Screen Recording grants may need renewing after every build.", file=sys.stderr)


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("--prepare", "--sign"):
        raise ValueError("Usage: macos-sign-bundle.py [--prepare|--sign] /path/to/ME.app")
    bundle = Path(sys.argv[2]).resolve()
    assert_stopped(bundle)
    identity = signing_identity(bundle)
    if sys.argv[1] == "--prepare":
        print(identity)
    else:
        sign_bundle(bundle, identity)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError, OSError) as error:
        sys.exit(str(error))
