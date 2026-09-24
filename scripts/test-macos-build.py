#!/usr/bin/env python3
"""Regression coverage for channel identity, atomic installs and vault migration."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("build", Path(__file__).with_name("macos-build.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class DevelopmentChannel(unittest.TestCase):
    def test_user_build_has_fixed_path_and_separate_data(self):
        app, data = build.channel_paths("dev", Path("/Users/test"), Path("/tmp/build"))
        self.assertEqual(app, Path("/Users/test/Applications/ME Dev.app"))
        self.assertEqual(data, Path("/Users/test/Library/Application Support/ME Dev"))
        preview, preview_data = build.channel_paths("preview", Path("/Users/test"), Path("/tmp/build"))
        self.assertNotEqual(app, preview)
        self.assertNotEqual(data, preview_data)

    def test_new_revision_keeps_identity_and_account_configuration(self):
        data = Path("/tmp/dev")
        first = build.channel_plist({}, "dev", {}, {"ME_ACCOUNT_API_URL":"https://accounts.example.test"}, data, "a", 1)
        second = build.channel_plist({}, "dev", first, {"ME_VAULT_DIR":"/tmp/accidental-other-vault", "PRIVATE_KEY":"synthetic"}, data, "b", 2)
        for key in ("CFBundleIdentifier", "CFBundleName", "CFBundleDisplayName", "LSEnvironment"):
            self.assertEqual(first[key], second[key])
        self.assertNotEqual(first["CFBundleVersion"], second["CFBundleVersion"])
        self.assertEqual(second["LSEnvironment"]["ME_VAULT_DIR"], "/tmp/dev/vault")
        self.assertNotIn("PRIVATE_KEY", second["LSEnvironment"])

    def test_three_channels_never_share_bundle_identity(self):
        identifiers = {build.channel_plist({}, c, {}, {}, Path("/tmp/data"), "test", 1)["CFBundleIdentifier"] for c in ("dev","preview","release")}
        self.assertEqual(len(identifiers), 3)

    def test_migration_preserves_all_account_slots_and_provider_state(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); source=root/"old"; destination=root/"new"
            files = {"vault/header.json":"synthetic", "vault/vault.db":"encrypted-test-data", "accounts/slot/vault/header.json":"other", "device-account.json":"selection", "codex-inbox/auth.json":"synthetic-not-a-token", "api.log":"excluded", "Start.command":"excluded"}
            for name, value in files.items():
                p=source/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(value)
            cache=source/"codex-inbox/tmp";cache.mkdir()
            (cache/"generated-link").symlink_to(root)
            (source/"codex-inbox/models_cache.json").write_text("regenerated")
            before=build.snapshot(source)
            self.assertEqual(build.migrate_data(source,destination),5)
            self.assertEqual(build.snapshot(destination),before)
            self.assertEqual(build.snapshot(source),before)
            self.assertFalse((destination/"api.log").exists())
            with self.assertRaises(ValueError): build.migrate_data(source,destination)

    def test_migration_refuses_links_and_empty_sources(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);source=root/"old";source.mkdir()
            with self.assertRaises(ValueError): build.migrate_data(source,root/"new")
            (source/"vault").symlink_to(root, target_is_directory=True)
            with self.assertRaises(ValueError): build.migrate_data(source,root/"new")

    def test_identity_failure_keeps_installed_app_untouched(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);old=root/"ME Dev.app";new=root/"stage";old.mkdir();new.mkdir()
            (old/"version").write_text("old");(new/"version").write_text("new")
            with patch.object(build.signer,"assert_stopped"), patch.object(build,"verify_manifest",side_effect=ValueError("changed")):
                with self.assertRaises(ValueError): build.install(new,old,{"app":"certificate"})
            self.assertEqual((old/"version").read_text(),"old")

    def test_successful_update_replaces_only_app_bundle(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);old=root/"ME Dev.app";new=root/"stage";old.mkdir();new.mkdir()
            (old/"version").write_text("old");(new/"version").write_text("new");(root/"vault").write_text("preserved")
            with patch.object(build.signer,"assert_stopped"), patch.object(build,"verify_manifest"):
                build.install(new,old,{"app":"certificate"})
            self.assertEqual((old/"version").read_text(),"new")
            self.assertEqual((root/"vault").read_text(),"preserved")


if __name__ == "__main__": unittest.main()
