//! Local password generation. Uniform sampling, with every selected class required.
use crate::{Error, Result};
use rand_core::RngCore;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasswordOptions {
    pub length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}
impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            length: 20,
            uppercase: true,
            lowercase: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: false,
        }
    }
}
impl PasswordOptions {
    pub const MIN_LENGTH: usize = 8;
    pub const MAX_LENGTH: usize = 128;
}

pub fn generate_credential_password() -> Result<Zeroizing<String>> {
    generate_password(PasswordOptions::default())
}
pub fn generate_password(options: PasswordOptions) -> Result<Zeroizing<String>> {
    if !(PasswordOptions::MIN_LENGTH..=PasswordOptions::MAX_LENGTH).contains(&options.length) {
        return Err(Error::Validation(
            "Choose a password length between 8 and 128.",
        ));
    }
    let groups: Vec<Vec<u8>> = [
        (options.uppercase, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ".as_slice()),
        (options.lowercase, b"abcdefghijklmnopqrstuvwxyz".as_slice()),
        (options.digits, b"0123456789".as_slice()),
        (options.symbols, b"!@#$%^&*()-_=+[]{};:,.?".as_slice()),
    ]
    .into_iter()
    .filter(|(enabled, _)| *enabled)
    .map(|(_, bytes)| {
        bytes
            .iter()
            .copied()
            .filter(|b| !options.exclude_ambiguous || !b"0O1Il".contains(b))
            .collect()
    })
    .collect();
    if groups.is_empty() {
        return Err(Error::Validation("Choose at least one character type."));
    }
    let alphabet: Vec<u8> = groups.iter().flatten().copied().collect();
    let limit = 256 / alphabet.len() * alphabet.len();
    // Rejection of entire candidates keeps the distribution uniform among valid passwords.
    for _ in 0..256 {
        let mut result = Zeroizing::new(String::with_capacity(options.length));
        while result.len() < options.length {
            let mut bytes = Zeroizing::new([0u8; 128]);
            rand_core::OsRng
                .try_fill_bytes(bytes.as_mut())
                .map_err(|_| Error::Validation("Couldn't generate a password. Try again."))?;
            for b in bytes.iter().copied().filter(|b| (*b as usize) < limit) {
                result.push(alphabet[b as usize % alphabet.len()] as char);
                if result.len() == options.length {
                    break;
                }
            }
        }
        if groups
            .iter()
            .all(|g| result.bytes().any(|b| g.contains(&b)))
        {
            return Ok(result);
        }
    }
    Err(Error::Validation(
        "Couldn't generate a password. Try again.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_selected_class_is_present_and_excluded_classes_never_appear() {
        for bits in 1..16 {
            for length in [8, 20, 128] {
                let options = PasswordOptions {
                    length,
                    uppercase: bits & 1 != 0,
                    lowercase: bits & 2 != 0,
                    digits: bits & 4 != 0,
                    symbols: bits & 8 != 0,
                    exclude_ambiguous: true,
                };
                let password = generate_password(options).unwrap();
                assert_eq!(password.len(), length);
                assert_eq!(
                    password.bytes().any(|b| b.is_ascii_uppercase()),
                    options.uppercase
                );
                assert_eq!(
                    password.bytes().any(|b| b.is_ascii_lowercase()),
                    options.lowercase
                );
                assert_eq!(password.bytes().any(|b| b.is_ascii_digit()), options.digits);
                assert_eq!(
                    password.bytes().any(|b| b.is_ascii_punctuation()),
                    options.symbols
                );
                assert!(!password.bytes().any(|b| b"0O1Il".contains(&b)));
            }
        }
    }
    #[test]
    fn rejects_empty_alphabets_and_out_of_range_lengths() {
        for length in [0, 7, 129, usize::MAX] {
            assert!(
                generate_password(PasswordOptions {
                    length,
                    ..Default::default()
                })
                .is_err()
            );
        }
        assert!(
            generate_password(PasswordOptions {
                uppercase: false,
                lowercase: false,
                digits: false,
                symbols: false,
                ..Default::default()
            })
            .is_err()
        );
    }
}
