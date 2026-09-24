#!/usr/bin/env python3
"""Regression checks for Finder-launched account builds."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("bundle_plist", Path(__file__).with_name("macos-bundle-plist.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
configure = module.configured_plist


class BundleLaunchSettings(unittest.TestCase):
    def test_rebuild_preserves_account_origin_and_vault_location(self):
        first = configure({"CFBundleVersion": "1"}, {}, {
            "ME_ACCOUNT_API_URL": "http://127.0.0.1:18787",
            "ME_VAULT_DIR": "/tmp/synthetic account/vault",
        })
        rebuilt = configure({"CFBundleVersion": "2"}, first, {})
        self.assertEqual(rebuilt["CFBundleVersion"], "2")
        self.assertEqual(rebuilt["LSEnvironment"], first["LSEnvironment"])

    def test_explicit_configuration_overrides_or_removes_saved_values(self):
        previous = {"LSEnvironment": {
            "ME_ACCOUNT_API_URL": "https://old.example.test",
            "ME_VAULT_DIR": "/tmp/old/vault",
        }}
        updated = configure({}, previous, {
            "ME_ACCOUNT_API_URL": "https://accounts.example.test",
            "ME_VAULT_DIR": "",
        })
        self.assertEqual(updated["LSEnvironment"], {
            "ME_ACCOUNT_API_URL": "https://accounts.example.test",
        })

    def test_no_other_environment_values_are_bundled(self):
        result = configure({}, {"LSEnvironment": {"PRIVATE_KEY": "synthetic"}}, {
            "ME_DATABASE_URL": "synthetic-private-database-setting",
            "OPENAI_API_KEY": "synthetic-not-a-key",
        })
        self.assertNotIn("LSEnvironment", result)

    def test_rejects_unsafe_or_invalid_settings(self):
        for value in [
            "http://accounts.example.test", "https://name:pass@example.test",
            "https://example.test/path", "https://example.test?key=value",
            "https://example.test#fragment", "https://example.test:99999",
            "https://example.test\n", "https:///", "file:///tmp/server",
        ]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                configure({}, {}, {"ME_ACCOUNT_API_URL": value})
        with self.assertRaises(ValueError):
            configure({}, {}, {"ME_VAULT_DIR": "relative/vault"})


if __name__ == "__main__":
    unittest.main()
