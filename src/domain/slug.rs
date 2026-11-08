use thiserror::Error;

const MAX_LEN: usize = 64;

/// A short lowercase name: ASCII letters, digits, `-` and `_`, at most 64 characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slug(String);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("`{0}` is not a lowercase name of letters, digits, `-` or `_` (at most 64 characters)")]
pub struct ParseError(pub String);

impl Slug {
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let valid = !text.is_empty()
            && text.len() <= MAX_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(ParseError(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Text with no whitespace, used for identifiers a provider or host assigns.
pub(crate) fn token(text: &str) -> Option<String> {
    (!text.is_empty() && !text.chars().any(char::is_whitespace)).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_lowercase_names() {
        assert_eq!(
            Slug::parse("azure-devops").unwrap().as_str(),
            "azure-devops"
        );
        assert!(Slug::parse("handed_off").is_ok());
    }

    #[test]
    fn rejects_uppercase_spaces_and_empty() {
        assert!(Slug::parse("GitHub").is_err());
        assert!(Slug::parse("a b").is_err());
        assert!(Slug::parse("").is_err());
        assert!(Slug::parse(&"a".repeat(65)).is_err());
    }
}
