#!/usr/bin/env python3
"""Retain explicit, non-secret app launch settings across macOS rebuilds."""
import os
import plistlib
import sys
from pathlib import Path
from urllib.parse import urlsplit


LAUNCH_SETTINGS = ("ME_ACCOUNT_API_URL", "ME_VAULT_DIR")


def configured_plist(template, previous, environment):
    result = dict(template)
    saved = previous.get("LSEnvironment", {})
    settings = {}
    for key in LAUNCH_SETTINGS:
        value = environment.get(key, saved.get(key))
        if value is None or value == "":
            continue
        if not isinstance(value, str) or any(ord(char) < 32 for char in value):
            raise ValueError(f"Invalid {key} launch setting")
        settings[key] = value
    if value := settings.get("ME_ACCOUNT_API_URL"):
        url = urlsplit(value)
        local = url.hostname in ("127.0.0.1", "::1", "localhost")
        if (
            not url.hostname
            or (url.scheme != "https" and not (url.scheme == "http" and local))
            or url.username is not None
            or url.password is not None
            or "?" in value
            or "#" in value
            or url.path not in ("", "/")
        ):
            raise ValueError("Account service must be an HTTPS origin (or loopback HTTP)")
        # Accessing .port also rejects malformed or out-of-range ports.
        _ = url.port
    if value := settings.get("ME_VAULT_DIR"):
        if not Path(value).is_absolute():
            raise ValueError("ME_VAULT_DIR must be an absolute path")
    result.pop("LSEnvironment", None)
    if settings:
        result["LSEnvironment"] = settings
    return result


def main():
    template_path, previous_path, output_path = map(Path, sys.argv[1:])
    template = plistlib.loads(template_path.read_bytes())
    previous = plistlib.loads(previous_path.read_bytes()) if previous_path.exists() else {}
    result = configured_plist(template, previous, os.environ)
    output_path.write_bytes(plistlib.dumps(result))
    if "ME_ACCOUNT_API_URL" not in result.get("LSEnvironment", {}):
        print("Note: this bundle has no account service; online sign-in requires a configured build.", file=sys.stderr)


if __name__ == "__main__":
    main()
