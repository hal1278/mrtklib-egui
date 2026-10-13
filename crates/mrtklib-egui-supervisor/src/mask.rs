//! The common masking point for GUI display and logging.

/// Mask non-empty secret values and password-bearing URL/stream-path userinfo.
/// Whitespace is preserved. Leading quotes and an assignment prefix (`path=`) are
/// kept outside userinfo; the last `@` before the first path slash ends userinfo.
/// A colon in userinfo means a password, so the entire userinfo becomes `***`.
/// Schema-marked configuration secrets must be supplied here once config RPC exists.
pub fn mask(text: &str, secrets: &[&str]) -> String {
    let mut userinfo_masked = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        userinfo_masked.push_str(&mask_word(word));
    }
    // Merge overlapping occurrences in the original text, rather than masking
    // replacement markers again or exposing the tail of an overlapping secret.
    let mut result = String::with_capacity(userinfo_masked.len());
    let mut rest = userinfo_masked.as_str();
    while !rest.is_empty() {
        if let Some(mut end) = secret_length(rest, secrets) {
            let mut cursor = 0;
            while cursor < end {
                let tail = &rest[cursor..];
                if let Some(length) = secret_length(tail, secrets) {
                    end = end.max(cursor + length);
                }
                cursor += tail
                    .chars()
                    .next()
                    .expect("inside matched secret")
                    .len_utf8();
            }
            result.push_str("***");
            rest = &rest[end..];
        } else {
            let ch = rest.chars().next().expect("non-empty text");
            result.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    result
}

fn secret_length(text: &str, secrets: &[&str]) -> Option<usize> {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty() && text.starts_with(**secret))
        .map(|secret| secret.len())
        .max()
}

fn mask_word(word: &str) -> String {
    let mut start = word.len() - word.trim_start_matches(['\'', '"']).len();
    let rest = &word[start..];
    if let Some(equal) = rest.find('=')
        && !rest[..equal].contains([':', '@', '/'])
    {
        start += equal + 1;
        start += word[start..].len() - word[start..].trim_start_matches(['\'', '"']).len();
    }
    if let Some(scheme) = word[start..].find("://")
        && word[start..start + scheme]
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    {
        start += scheme + 3;
    }
    let authority = word[start..].split('/').next().expect("one part");
    if let Some(at) = authority.rfind('@')
        && authority[..at].contains(':')
    {
        format!("{}***{}", &word[..start], &word[start + at..])
    } else {
        word.into()
    }
}

#[cfg(test)]
mod tests {
    use super::mask;

    #[test]
    fn userinfo_and_surrounding_text() {
        for (input, expected) in [
            (
                "ntrip://user:pass@caster:2101/MNT",
                "ntrip://***@caster:2101/MNT",
            ),
            (
                "user:pass@caster.example:2101/MNT",
                "***@caster.example:2101/MNT",
            ),
            ("ftp://user:p@ss@host/path", "ftp://***@host/path"),
            ("user@host", "user@host"),
            ("'user:pass@host'", "'***@host'"),
            ("path=user:pass@host", "path=***@host"),
            (
                "path=\"ntrip://user:pa=ss@host/a\"",
                "path=\"ntrip://***@host/a\"",
            ),
            ("a\tuser:pass@host\n  b", "a\t***@host\n  b"),
            (
                "http://host/path/user:pass@other",
                "http://host/path/user:pass@other",
            ),
        ] {
            assert_eq!(mask(input, &[]), expected, "{input}");
        }
    }

    #[test]
    fn secrets_inside_words_repeated_and_overlapping() {
        assert_eq!(
            mask("prefixTOKENsuffix TOKEN", &["", "TOKEN"]),
            "prefix***suffix ***"
        );
        assert_eq!(mask("abcdef abc", &["abc", "abcdef"]), "*** ***");
        assert_eq!(mask("abcdef", &["abc", "cde"]), "***f");
        assert_eq!(mask("ababa", &["aba"]), "***");
        assert_eq!(mask("秘密suffix", &["秘密"]), "***suffix");
    }
}
