//! Dependency-free token-cost estimation for skill descriptions.
//!
//! Agent harnesses keep every linked skill's name and description in context,
//! so description length is a real, always-on cost that users cannot see. The
//! number only has to drive "keep or disable this skill" decisions, so a cheap
//! heuristic beats pulling in a full tokenizer (whose vocabulary would only be
//! right for one model family anyway).

/// Estimate the token count of `text`.
///
/// Heuristic — an order-of-magnitude estimate, not an exact count, because real
/// tokenizers vary per model:
///
/// - non-ASCII characters (CJK, emoji, ...) count as one token each;
/// - ASCII characters count as one token per four characters, rounded up.
///
/// The two counts are added; empty text estimates to zero.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut ascii = 0u32;
    let mut non_ascii = 0u32;
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.div_ceil(4) + non_ascii
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_costs_nothing() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn ascii_rounds_up_per_four_characters() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("does pdf"), 2);
    }

    #[test]
    fn non_ascii_counts_one_token_per_character() {
        assert_eq!(estimate_tokens("中文描述"), 4);
        // Mixed scripts add up: 4 ASCII chars (1 token) + 2 CJK chars (2 tokens).
        assert_eq!(estimate_tokens("pdf 中文"), 3);
    }
}
