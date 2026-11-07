//! Random password generation.

use serde::{Deserialize, Serialize};

/// Characters that are easy to confuse with each other in common fonts.
pub const AMBIGUOUS: &str = "0Oo1lI|5S8B2Z";
/// Readable subset of punctuation.
pub const SYMBOLS: &str = "!@#$%&*?-_=+";

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenOptions {
    pub length: usize,
    pub digits: bool,
    pub symbols: bool,
    pub uppercase: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            length: 16,
            digits: true,
            symbols: false,
            uppercase: true,
            exclude_ambiguous: true,
        }
    }
}

/// Enabled character classes, lowercase first. Each class has ambiguous characters
/// removed when `exclude_ambiguous` is set.
fn classes(opts: &GenOptions) -> Vec<Vec<char>> {
    let filter = |s: &str| -> Vec<char> {
        s.chars()
            .filter(|c| !opts.exclude_ambiguous || !AMBIGUOUS.contains(*c))
            .collect()
    };
    let mut out = vec![filter(LOWER)];
    if opts.uppercase {
        out.push(filter(UPPER));
    }
    if opts.digits {
        out.push(filter(DIGITS));
    }
    if opts.symbols {
        out.push(filter(SYMBOLS));
    }
    out.retain(|c| !c.is_empty());
    out
}

/// The exact character set `generate` draws from.
pub fn alphabet(opts: &GenOptions) -> String {
    classes(opts).into_iter().flatten().collect()
}

/// Generates a password. When `length` is at least the number of enabled classes,
/// the result contains at least one character from each class.
pub fn generate(opts: &GenOptions) -> String {
    if opts.length == 0 {
        return String::new();
    }
    let classes = classes(opts);
    let all: Vec<char> = classes.iter().flatten().copied().collect();
    let mut chars: Vec<char> = Vec::with_capacity(opts.length);
    if opts.length >= classes.len() {
        for class in &classes {
            chars.push(class[random_below(class.len())]);
        }
    }
    while chars.len() < opts.length {
        chars.push(all[random_below(all.len())]);
    }
    // Fisher–Yates, so the one-per-class characters do not sit at the start.
    for i in (1..chars.len()).rev() {
        chars.swap(i, random_below(i + 1));
    }
    chars.into_iter().collect()
}

/// Uniform number in `0..n` (n > 0) from the operating system's random source. Values from
/// the top of the 32-bit range that would make small numbers likelier are drawn again.
fn random_below(n: usize) -> usize {
    let n = n as u64;
    let range = 1u64 << 32;
    let limit = range - range % n;
    loop {
        let r = u64::from(getrandom::u32().expect("the operating system's random source failed"));
        if r < limit {
            return (r % n) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_alphabet_has_no_ambiguous_chars() {
        let a = alphabet(&GenOptions::default());
        assert!(a.chars().all(|c| !AMBIGUOUS.contains(c)), "{a}");
        assert!(a.contains('a') && a.contains('A') && a.contains('3'));
    }

    #[test]
    fn alphabet_with_everything_and_ambiguous_allowed() {
        let opts = GenOptions { symbols: true, exclude_ambiguous: false, ..Default::default() };
        let a = alphabet(&opts);
        assert_eq!(a.len(), 26 + 26 + 10 + SYMBOLS.len());
        assert!(a.contains('0') && a.contains('O') && a.contains('l'));
    }

    #[test]
    fn every_enabled_class_present() {
        let opts = GenOptions { symbols: true, ..Default::default() };
        for _ in 0..200 {
            let p = generate(&opts);
            assert_eq!(p.chars().count(), 16);
            assert!(p.chars().any(|c| c.is_ascii_digit()), "{p}");
            assert!(p.chars().any(|c| SYMBOLS.contains(c)), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_uppercase()), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_lowercase()), "{p}");
            assert!(p.chars().all(|c| !AMBIGUOUS.contains(c)), "{p}");
        }
    }

    #[test]
    fn length_honored() {
        for len in [0usize, 1, 2, 3, 7, 64] {
            let opts = GenOptions { length: len, symbols: true, ..Default::default() };
            assert_eq!(generate(&opts).chars().count(), len);
        }
    }

    #[test]
    fn lowercase_only() {
        let opts = GenOptions { digits: false, uppercase: false, symbols: false, ..Default::default() };
        let p = generate(&opts);
        assert!(p.chars().all(|c| c.is_ascii_lowercase()), "{p}");
    }

    #[test]
    fn random_below_stays_in_range_and_reaches_every_value() {
        assert_eq!(random_below(1), 0);
        for n in [2usize, 3, 7, 62] {
            let mut seen = vec![0u32; n];
            for _ in 0..n * 200 {
                let r = random_below(n);
                assert!(r < n, "{r} >= {n}");
                seen[r] += 1;
            }
            // About 200 each; a value never drawn would mean a broken mapping.
            assert!(seen.iter().all(|&c| c > 100), "n={n}: {seen:?}");
        }
    }
}
