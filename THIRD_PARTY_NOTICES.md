# Third-party notices

ME.'s [root license](LICENSE) applies to the project's original material. It
does not replace the licenses of dependencies, bundled fonts, or other
third-party material. Those components may grant permissions independently of
ME.'s commercial licensing terms.

## Bundled fonts and retained notices

| Component | License / notice in this repository |
| --- | --- |
| Geist and Geist Mono, copyright 2024 The Geist Project Authors | [SIL Open Font License 1.1](apps/desktop/assets/fonts/OFL.txt) |
| GPUI | [Apache License 2.0](apps/desktop/assets/licenses/GPUI-APACHE-2.0.txt) |
| SQLCipher | [SQLCipher copyright and license](apps/desktop/assets/licenses/SQLCipher.txt) |
| OpenSSL | [OpenSSL license](apps/desktop/assets/licenses/OpenSSL.txt) |
| `hashify` 0.2.9 | [MIT license](apps/desktop/assets/licenses/hashify-0.2.9-MIT.txt) |
| Rust `curl` / `curl-sys` bindings | [MIT license](apps/desktop/assets/licenses/curl-rust-MIT.txt) |
| libcurl | [curl license](apps/desktop/assets/licenses/libcurl.txt) |
| `mail-parser` 0.11.3 | [MIT license](apps/desktop/assets/licenses/mail-parser-0.11.3-MIT.txt) |

The font files are distributed in the source tree under the OFL. The dependency
source code is resolved by Cargo rather than vendored here. The macOS development
bundle copies the retained notices and font license into `Contents/Resources/Licenses`.

## Dependency and distribution scope

`Cargo.lock` records exact Rust dependency versions. Each dependency's upstream
package contains its license expression, notices, and any separately licensed
subcomponents. The table above is an index of notices currently retained in this
repository, **not a complete license inventory for a compiled distribution**.

Some locked dependencies use MPL-2.0, including `cbindgen`, `dwrote`, and
`option-ext`; their inclusion depends on platform and build graph. Their source
and distribution obligations remain under the MPL and are not replaced by the
ME. license. Alternative-license dependencies retain their stated choices.

Before distributing binaries, inspect the dependency graph for each target,
collect all required copyright and license texts, and satisfy any corresponding
source obligations. The current development bundler does not perform that full
release-compliance process automatically.

Apple frameworks are supplied by the operating system. Linux document tools
(Poppler, Tesseract, and UnRTF), Rust/Xcode, and the Codex CLI are separately
installed software, not re-licensed by this repository.
