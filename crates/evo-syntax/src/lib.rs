//! Lexer and lossless, error-tolerant parser for the Evolve language.
//!
//! Implements the draft of [RFC 0001](../../../rfcs/0001-syntax-core.md) and
//! `spec/grammar.ebnf`. The tree is a rowan CST: `parse(src).syntax().to_string()`
//! always reproduces `src` exactly, even when the source has errors, so evolved
//! variants can be edited on the tree and printed back as readable code.
//!
//! ```
//! let parse = evo_syntax::parse("ys = sin.(xs) .+ 1\n");
//! assert!(parse.errors().is_empty());
//! assert_eq!(parse.syntax().to_string(), "ys = sin.(xs) .+ 1\n");
//! ```

mod lexer;
mod parser;
mod syntax_kind;

use rowan::{GreenNode, TextRange};

pub use syntax_kind::{EvoLanguage, SyntaxKind};

pub type SyntaxNode = rowan::SyntaxNode<EvoLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<EvoLanguage>;

/// A syntax error with the source range it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub range: TextRange,
}

impl SyntaxError {
    pub(crate) fn new(message: impl Into<String>, range: TextRange) -> Self {
        SyntaxError {
            message: message.into(),
            range,
        }
    }
}

/// The result of parsing: a complete tree plus any errors found on the way.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<SyntaxError>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    /// Errors as `line:col: message`, one per line (1-based).
    pub fn render_errors(&self, src: &str) -> String {
        self.errors
            .iter()
            .map(|e| {
                let offset = usize::from(e.range.start()).min(src.len());
                let before = &src[..offset];
                let line = before.matches('\n').count() + 1;
                let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                format!("{line}:{col}: {}", e.message)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The full tree with kinds and ranges, for debugging and snapshots.
    pub fn debug_tree(&self) -> String {
        format!("{:#?}", self.syntax())
    }
}

/// Parses an Evolve source file. Never fails: errors are reported in [`Parse::errors`].
pub fn parse(src: &str) -> Parse {
    let mut errors = Vec::new();
    let tokens = lexer::lex(src, &mut errors);
    let (green, errors) = parser::Parser::new(src, tokens, errors).parse_file();
    Parse { green, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compact S-expression of the first statement's expression, without trivia.
    fn sexp(src: &str) -> String {
        let parse = parse(src);
        assert!(
            parse.errors().is_empty(),
            "{src}: {}",
            parse.render_errors(src)
        );
        let stmt = parse.syntax().first_child().expect("a statement");
        let expr = stmt.first_child().expect("an expression");
        render(&expr)
    }

    /// Brackets and commas are dropped and single-child nodes collapse, so
    /// `f(b)` renders as `(f b)` and the structure stays readable.
    fn render(node: &SyntaxNode) -> String {
        use SyntaxKind::*;
        let parts: Vec<String> = node
            .children_with_tokens()
            .filter(|el| {
                !el.kind().is_trivia()
                    && !matches!(el.kind(), L_PAREN | R_PAREN | L_BRACK | R_BRACK | COMMA)
            })
            .map(|el| match el {
                rowan::NodeOrToken::Node(n) => render(&n),
                rowan::NodeOrToken::Token(t) => t.text().to_string(),
            })
            .collect();
        if parts.len() == 1 {
            parts[0].clone()
        } else {
            format!("({})", parts.join(" "))
        }
    }

    #[test]
    fn arithmetic_precedence() {
        assert_eq!(sexp("1 + 2 * 3\n"), "(1 + (2 * 3))");
        assert_eq!(sexp("a - b - c\n"), "((a - b) - c)");
    }

    #[test]
    fn power_is_right_associative_and_binds_tighter_than_unary_minus() {
        assert_eq!(sexp("2 ^ 3 ^ 2\n"), "(2 ^ (3 ^ 2))");
        assert_eq!(sexp("-x ^ 2\n"), "(- (x ^ 2))");
    }

    #[test]
    fn logic_and_comparison() {
        assert_eq!(sexp("a or b and c\n"), "(a or (b and c))");
        assert_eq!(sexp("not a == b\n"), "(not (a == b))");
        assert_eq!(sexp("x not in xs\n"), "(x not in xs)");
        assert_eq!(sexp("x is not none\n"), "(x is not none)");
    }

    #[test]
    fn pipe_is_lowest_and_left_associative() {
        assert_eq!(sexp("a |> f(b) |> g\n"), "((a |> (f b)) |> g)");
        assert_eq!(sexp("x |> f or y\n"), "(x |> (f or y))");
    }

    #[test]
    fn broadcasting() {
        assert_eq!(sexp("a .+ b .* c\n"), "(a .+ (b .* c))");
        assert_eq!(sexp("sin.(x)\n"), "(sin . x)");
    }

    #[test]
    fn ranges_uncertainty_and_units() {
        assert_eq!(sexp("0..n + 1\n"), "(0 .. (n + 1))");
        assert_eq!(sexp("9.81 ± 0.02 m/s²\n"), "(9.81 ± (0.02 (m / s ²)))");
        assert_eq!(sexp("5 N*m\n"), "(5 (N * m))");
    }

    #[test]
    fn lambdas_and_placeholders() {
        assert_eq!(sexp("x -> x + 1\n"), "(x -> (x + 1))");
        assert_eq!(sexp("(a, b) -> a\n"), "((a b) -> a)");
        assert_eq!(
            sexp("filter(_.region == \"EU\")\n"),
            "(filter ((_ . region) == \"EU\"))"
        );
    }

    #[test]
    fn evolvable_constant_with_scale_hint() {
        assert_eq!(
            sexp("~0.001 in 1e-5..1e-1 log\n"),
            "(~ 0.001 in (1e-5 .. 1e-1) log)"
        );
    }

    #[test]
    fn symbols_slices_and_string_macros() {
        assert_eq!(sexp("groupby(:month)\n"), "(groupby (: month))");
        assert_eq!(sexp("df[:, :period]\n"), "(df (: (: period)))");
        assert_eq!(sexp("xs[1:3]\n"), "(xs (1 : 3))");
        assert_eq!(sexp("read\"x.csv\"\n"), "(read \"x.csv\")");
    }

    #[test]
    fn indentation_produces_blocks() {
        let src = "if x:\n    y = 1\n    z = 2\nw = 3\n";
        let parse = parse(src);
        assert!(parse.errors().is_empty(), "{}", parse.render_errors(src));
        let kinds: Vec<_> = parse.syntax().children().map(|n| n.kind()).collect();
        assert_eq!(kinds, [SyntaxKind::IF_STMT, SyntaxKind::ASSIGN_STMT]);
    }

    #[test]
    fn pipeline_lines_join() {
        let src = "r = xs\n  |> f\n  # note\n  |> g\n";
        let parse = parse(src);
        assert!(parse.errors().is_empty(), "{}", parse.render_errors(src));
        assert_eq!(parse.syntax().children().count(), 1);
    }

    #[test]
    fn conditional_binds_looser_than_pipe_and_tighter_than_lambda() {
        assert_eq!(
            sexp("xs |> mean if xs else 0.0\n"),
            "((xs |> mean) if xs else 0.0)"
        );
        assert_eq!(sexp("x -> a if c else b\n"), "(x -> (a if c else b))");
        assert_eq!(
            sexp("a if p else b if q else c\n"),
            "(a if p else (b if q else c))"
        );
    }

    #[test]
    fn power_takes_a_signed_exponent() {
        assert_eq!(sexp("2 ^ -1\n"), "(2 ^ (- 1))");
    }

    #[test]
    fn comprehensions_starred_and_floor_division() {
        assert_eq!(
            sexp("[x for x in xs if x > 0]\n"),
            "(x (for x in xs) (if (x > 0)))"
        );
        assert_eq!(sexp("sum(x for x in xs)\n"), "(sum (x (for x in xs)))");
        assert_eq!(sexp("first, *rest = xs\n"), "(first (* rest))");
        assert_eq!(sexp("a // b * c\n"), "((a // b) * c)");
    }

    #[test]
    fn one_direction_comparison_chains_are_fine() {
        assert_eq!(sexp("0 <= p <= 1\n"), "((0 <= p) <= 1)");
        assert_eq!(sexp("a == b == c\n"), "((a == b) == c)");
    }

    /// Soft keywords lex as names and are re-tagged only in their declaration position.
    #[test]
    fn soft_keywords() {
        use SyntaxKind::*;
        let src =
            "model = 1\nwith(x)\ngene def f(): ...\nwith open(p) as f: pass\nmodel m:\n    pass\n";
        let parse = parse(src);
        let kinds: Vec<_> = parse.syntax().children().map(|n| n.kind()).collect();
        assert_eq!(kinds, [ASSIGN_STMT, EXPR_STMT, GENE_DEF, WITH_STMT, ERROR]);
        let first_token = |n: SyntaxNode| n.first_token().map(|t| t.kind());
        let nodes: Vec<_> = parse.syntax().children().collect();
        assert_eq!(first_token(nodes[0].clone()), Some(IDENT));
        assert_eq!(first_token(nodes[2].clone()), Some(GENE_KW));
        assert_eq!(first_token(nodes[3].clone()), Some(WITH_KW));
        assert_eq!(parse.errors().len(), 1, "{}", parse.render_errors(src));
        assert!(parse.errors()[0].message.contains("soft keyword"));
    }

    fn messages(src: &str) -> Vec<String> {
        let parse = parse(src);
        assert_eq!(parse.syntax().to_string(), src, "round-trip");
        parse.errors().iter().map(|e| e.message.clone()).collect()
    }

    #[test]
    fn byte_order_mark_only_at_the_start() {
        assert!(messages("\u{feff}x = 1\n").is_empty());
        assert!(messages("\u{feff}if x:\n    y = 1\n").is_empty());
        assert!(messages("x = 1\u{feff}\n")[0].contains("byte-order mark"));
    }

    #[test]
    fn lone_carriage_return_is_an_error_but_still_ends_the_line() {
        let src = "x = 1\ry = 2\n";
        assert_eq!(messages(src).len(), 1);
        assert!(messages(src)[0].contains("carriage return"));
        assert_eq!(parse(src).syntax().children().count(), 2);
        assert!(messages("x = 1\r\ny = 2\r\n").is_empty());
    }

    /// RFC 0001 "Strings": `\` followed by a newline is an escape, and a line ends
    /// in LF or CRLF.
    #[test]
    fn backslash_newline_in_a_string_works_with_lf_and_crlf() {
        for src in [
            "s = \"a\\\nb\"\n",
            "s = \"a\\\r\nb\"\r\n",
            "s = f\"a\\\r\nb\"\r\n",
            "s = b'a\\\r\nb'\r\n",
        ] {
            let found = messages(src);
            assert!(found.is_empty(), "{src:?}: {found:?}");
        }
    }

    #[test]
    fn bidi_controls_are_errors_even_in_strings_and_comments() {
        for src in [
            "s = \"a\u{202e}b\"\n",
            "x = 1  # \u{2066}\n",
            "x\u{202d} = 1\n",
        ] {
            let found = messages(src);
            assert_eq!(found.len(), 1, "{src:?}: {found:?}");
            assert!(found[0].contains("bidirectional"));
        }
        assert!(messages("s = \"a\\u202eb\"\n").is_empty());
    }

    /// RFC 0001 "Source text": the NUL character is an error anywhere.
    #[test]
    fn nul_is_an_error_even_in_strings_and_comments() {
        for src in [
            "x = 1 \0\n",
            "s = \"a\0b\"\n",
            "s = r\"a\0b\"\n",
            "s = f\"a\0b\"\n",
            "x = 1  # a\0b\n",
        ] {
            let found = messages(src);
            assert_eq!(found.len(), 1, "{src:?}: {found:?}");
            assert!(found[0].contains("NUL"), "{src:?}: {found:?}");
        }
        assert!(messages("s = \"a\\0b\"\n").is_empty());
    }

    #[test]
    fn superscripts_never_join_a_name() {
        let kinds: Vec<_> = lexer::raw_lex("x²", &mut Vec::new())
            .iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(kinds, [SyntaxKind::IDENT, SyntaxKind::SUPERSCRIPT]);
    }

    #[test]
    fn python_spellings_get_hints() {
        let parse = parse("x = 2 ** 8\n");
        assert!(parse.errors()[0].message.contains("use `^`"));
    }
}
