#!/usr/bin/env python3
"""Permission-identity regressions, without touching the keychain or TCC."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("bundle_sign", Path(__file__).with_name("macos-sign-bundle.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
A = ("A" * 40, "Developer ID Application: Test Owner (TEAMONE)")
B = ("B" * 40, "Developer ID Application: Other Owner (TEAMTWO)")
D = ("D" * 40, "Apple Development: Test Owner (TEAMONE)")


class PermissionIdentity(unittest.TestCase):
    def test_certificate_selection_and_parser(self):
        parsed = module.identities(f'  1) {A[0]} "{A[1]}"\n  2) {D[0]} "{D[1]}"\n  2 valid identities found\n')
        self.assertEqual(parsed, [A, D])
        self.assertEqual(module.choose_identity(parsed), A[0])
        self.assertEqual(module.choose_identity([D]), D[0])

    def test_rebuild_reuses_previous_signer(self):
        self.assertEqual(module.choose_identity([A, B], previous_authority=A[1]), A[0])
        for available in [[], [B]]:
            with self.assertRaises(ValueError):
                module.choose_identity(available, previous_authority=A[1])

    def test_ambiguous_or_missing_identity_fails_instead_of_ad_hoc(self):
        for available in [[], [A, B]]:
            with self.assertRaises(ValueError):
                module.choose_identity(available)

    def test_explicit_identity_requires_exact_name_or_hash(self):
        self.assertEqual(module.choose_identity([A, B], B[1]), B[0])
        self.assertEqual(module.choose_identity([A, B], B[0].lower()), B[0])
        with self.assertRaises(ValueError):
            module.choose_identity([A, B], "Developer ID Application")

    def test_cannot_downgrade_certificate_signed_app(self):
        with self.assertRaises(ValueError):
            module.choose_identity([A], "-", A[1])
        self.assertEqual(module.choose_identity([], "-"), "-")

    def test_running_bundle_cannot_be_replaced(self):
        bundle = Path("/tmp/ME Permission Test.app")
        with self.assertRaises(ValueError):
            module.assert_stopped(bundle, f" /other/app\n{bundle}/Contents/MacOS/me\n")
        module.assert_stopped(bundle, "/other/app\n/tmp/ME Permission Test.app/Contents/MacOS/me-mcp\n")


if __name__ == "__main__":
    unittest.main()
