//! Rewriting both renderings' numerals into one spelling (ADR 4.10.26b).
//!
//! The bootstrap lowers `2` to `(succ (succ zero))`; the self-host keeps a
//! `NatLit` and renders `2`. The terms are convertible, the strings are not,
//! so a byte comparison reports a divergence on every definition that mentions
//! a numeral. This module gives the comparison a second spelling to try.
//!
//! It works on the rendered *text*, because the self-host's term only ever
//! arrives as a string — and text cannot tell the constructor `zero` from a
//! variable of that name, or from the word inside a string literal. So it
//! declines (D2a) rather than guessing: a claimed pass is one where the rewrite
//! was unambiguous.

/// What opens one link of a successor chain, exactly as `Display for Term`
/// writes `Term::Succ`.
const SUCC_OPEN: &str = "(succ ";

/// The constructor every chain ends in.
const ZERO: &str = "zero";

/// Renderings in which `zero` or `succ` is a *bound name*, so a token spelled
/// that way may be a variable rather than the constructor. Each is the prefix
/// `Display for Term` writes before a binder: lambda, fix, let, the `natind`
/// motive's index, `case` arms and `adt_match` arms.
const BINDER_SPELLINGS: &[&str] = &[
    "λzero:",
    "λsucc:",
    "fix zero:",
    "fix succ:",
    "let zero ",
    "let succ ",
    "[zero.",
    "[succ.",
    "inl zero ",
    "inl succ ",
    "inr zero ",
    "inr succ ",
    " zero =>",
    " succ =>",
];

/// Rewrite every maximal `(succ … zero)` chain of depth *n* to `n`, and every
/// bare `zero` token to `0`.
///
/// `None` when the text is ambiguous — it holds a string literal, or binds
/// `zero`/`succ` as a name — in which case the caller must not claim the two
/// renderings agree on any reading but the literal one.
#[must_use]
pub fn normalise_numerals(rendering: &str) -> Option<String> {
    if rendering.contains('"')
        || BINDER_SPELLINGS
            .iter()
            .any(|binder| rendering.contains(binder))
    {
        return None;
    }
    let mut normalised = String::with_capacity(rendering.len());
    let mut rest = rendering;
    let mut previous: Option<char> = None;
    while let Some(next) = rest.chars().next() {
        if let Some((depth, consumed)) = succ_chain_at(rest) {
            normalised.push_str(&depth.to_string());
            rest = &rest[consumed..];
            previous = Some(')');
            continue;
        }
        if previous.is_none_or(is_token_boundary) && bare_zero_at(rest) {
            normalised.push('0');
            rest = &rest[ZERO.len()..];
            previous = Some('o');
            continue;
        }
        normalised.push(next);
        rest = &rest[next.len_utf8()..];
        previous = Some(next);
    }
    Some(normalised)
}

/// The depth of a `(succ (succ … zero))` chain starting at the head of `text`,
/// and how many bytes it spans. `None` when the head is not a complete chain —
/// `(succ n)` over a variable, or an unbalanced tail.
fn succ_chain_at(text: &str) -> Option<(usize, usize)> {
    let mut depth = 0;
    let mut cursor = text;
    while let Some(after) = cursor.strip_prefix(SUCC_OPEN) {
        depth += 1;
        cursor = after;
    }
    if depth == 0 || !bare_zero_at(cursor) {
        return None;
    }
    let closers = &cursor[ZERO.len()..];
    if closers.len() < depth || !closers.as_bytes()[..depth].iter().all(|b| *b == b')') {
        return None;
    }
    Some((depth, text.len() - closers.len() + depth))
}

/// Whether `text` opens with a `zero` token — the word followed by a boundary
/// or the end, so `zero_mul` is left alone. The *preceding* boundary is the
/// caller's to check.
fn bare_zero_at(text: &str) -> bool {
    text.strip_prefix(ZERO)
        .is_some_and(|after| after.chars().next().is_none_or(is_token_boundary))
}

/// What may sit either side of a token in a rendering: whitespace, a
/// parenthesis, a bracket or a pair's comma.
fn is_token_boundary(character: char) -> bool {
    character.is_whitespace() || matches!(character, '(' | ')' | '[' | ']' | ',')
}
