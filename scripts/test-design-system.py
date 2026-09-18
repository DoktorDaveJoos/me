#!/usr/bin/env python3
"""Regression checks for the guard: a new screen must not bypass shared tokens."""
import runpy
import tempfile
import unittest
from pathlib import Path

GUARD = runpy.run_path(str(Path(__file__).with_name('check-design-system')))
check = GUARD['rust_violations']


class DesignGuardTests(unittest.TestCase):
    def test_rejects_local_style_drift(self):
        for source in [
            'div().rounded(px(7.))', 'div().rounded_lg()',
            'div().text_size(px(17.))', 'div().text_sm()',
            'div().line_height(px(21.))', 'div().font_family("Arial")',
            'div().font_weight(FontWeight::BOLD)', 'div().gap(px(9.))',
            'div().px_4()', 'div().bg(rgb(0xffffff))',
            'const NEW_RED: u32 = 0xff0000;', 'svg().path("other.svg")',
        ]:
            with self.subTest(source=source):
                self.assertTrue(check(source))

    def test_accepts_tokens_geometry_and_text(self):
        self.assertFalse(check('''
            div().w(px(486.)).mx_auto().p(px(space::XXL))
                .type_style(Type::Body).font_family(font::MONO)
                .rounded(px(radius::STANDARD)).bg(rgb(SURFACE))
                .child(icon(Icon::Search, IconSize::Medium, MUTED));
            // .gap(px(9.)) is prohibited in executable UI code.
            let example = ".text_size(px(17.))";
            let height = window.line_height();
        '''))

    def test_reports_source_line_after_multiline_strings(self):
        self.assertEqual(check('let text = "first\nsecond";\ndiv().gap(px(9.));')[0][0], 3)

    def test_only_token_source_can_define_type_metrics(self):
        self.assertFalse(check('self.text_size(px(size)).line_height(px(leading))', tokens=True))
        self.assertTrue(check('self.text_size(px(size)).line_height(px(leading))'))

    def test_rejects_inconsistent_icon_strokes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'new.svg'
            path.write_text('<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path stroke-width="2"/></svg>')
            self.assertTrue(GUARD['icon_violations'](path))
            path.write_text(path.read_text().replace(' stroke-width="2"', ''))
            self.assertFalse(GUARD['icon_violations'](path))


if __name__ == '__main__':
    unittest.main()
