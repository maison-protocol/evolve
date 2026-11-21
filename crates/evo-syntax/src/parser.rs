//! Hand-written recursive-descent parser producing a lossless rowan tree.
//!
//! Error tolerance: the parser never stops. Unexpected input is wrapped in
//! `ERROR` nodes, an error is recorded, and parsing resumes at the next line.

use rowan::{Checkpoint, GreenNode, GreenNodeBuilder};

use crate::{
    SyntaxError, SyntaxKind,
    SyntaxKind::*,
    lexer::{Token, range},
};

pub(crate) struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<SyntaxError>,
    /// Set while parsing the upper bound of an evolvable range, where a trailing
    /// name is a scale hint (`~0.1 in 1e-5..1e-1 log`), not a unit.
    no_units: bool,
    /// Inside an `always` block (and not in a nested `def`): `return` would discard
    /// the failure in flight (RFC 0001).
    in_always: bool,
    /// Inside a loop that is itself inside the `always` block, so `break` and
    /// `continue` leave that loop, not the `always` block.
    loop_in_always: bool,
}

const FITNESS_INPUTS: &[&str] = &["examples", "holdout", "benchmark", "simulate", "replay"];

/// Soft keywords reserved for declaration blocks that later RFCs will specify.
const RESERVED_DECLS: &[&str] = &[
    "model",
    "agent",
    "protocol",
    "society",
    "app",
    "population",
    "species",
    "pipeline",
];

impl<'a> Parser<'a> {
    pub(crate) fn new(src: &'a str, tokens: Vec<Token>, errors: Vec<SyntaxError>) -> Self {
        Parser {
            src,
            tokens,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            errors,
            no_units: false,
            in_always: false,
            loop_in_always: false,
        }
    }

    pub(crate) fn parse_file(mut self) -> (GreenNode, Vec<SyntaxError>) {
        // Open the root before any trivia so leading comments stay inside it.
        self.builder.start_node(SOURCE_FILE.into());
        while !self.at(EOF) {
            if self.at(DEDENT) || self.at(NEWLINE) {
                self.bump(); // only after an indentation error, already reported
                continue;
            }
            self.statement();
        }
        self.flush_trivia();
        self.finish();
        (self.builder.finish(), self.errors)
    }

    // ------------------------------------------------------------ token cursor

    fn nth_index(&self, n: usize) -> Option<usize> {
        self.tokens[self.pos..]
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.kind.is_trivia())
            .nth(n)
            .map(|(i, _)| self.pos + i)
    }

    fn nth(&self, n: usize) -> SyntaxKind {
        self.nth_index(n).map_or(EOF, |i| self.tokens[i].kind)
    }

    fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        kinds.contains(&self.current())
    }

    fn current_text(&self) -> &'a str {
        self.nth_index(0)
            .map_or("", |i| &self.src[self.tokens[i].start..self.tokens[i].end])
    }

    fn at_contextual(&self, word: &str) -> bool {
        self.at(IDENT) && self.current_text() == word
    }

    /// True when the token after the current one follows it with no trivia between.
    fn next_is_adjacent(&self) -> bool {
        self.nth_index(0)
            .is_some_and(|i| self.tokens.get(i + 1).is_some_and(|t| !t.kind.is_trivia()))
    }

    fn at_line_end(&self) -> bool {
        self.at_any(&[NEWLINE, EOF, DEDENT])
    }

    fn is_name(kind: SyntaxKind) -> bool {
        kind == IDENT || kind.is_keyword()
    }

    // ------------------------------------------------------------ tree building

    fn flush_trivia(&mut self) {
        while let Some(tok) = self.tokens.get(self.pos).copied() {
            if !tok.kind.is_trivia() {
                break;
            }
            self.emit(tok);
            self.pos += 1;
        }
    }

    fn emit(&mut self, tok: Token) {
        if !tok.is_virtual() {
            self.builder
                .token(tok.kind.into(), &self.src[tok.start..tok.end]);
        }
    }

    fn bump(&mut self) {
        self.flush_trivia();
        if let Some(tok) = self.tokens.get(self.pos).copied() {
            self.emit(tok);
            self.pos += 1;
        }
    }

    /// Bumps the current token under another kind: soft keywords lex as IDENT and
    /// become keywords only where the grammar expects them.
    fn bump_remap(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        if let Some(tok) = self.tokens.get(self.pos).copied() {
            self.emit(Token { kind, ..tok });
            self.pos += 1;
        }
    }

    /// Kinds of the significant tokens from the current one to the end of the logical line.
    fn rest_of_line(&self) -> impl Iterator<Item = (usize, SyntaxKind)> + '_ {
        self.tokens[self.pos..]
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.kind.is_trivia())
            .map(|(i, t)| (self.pos + i, t.kind))
            .take_while(|&(_, k)| !matches!(k, NEWLINE | EOF | INDENT | DEDENT))
    }

    /// True when the logical line has a `:` outside brackets that is not a symbol.
    /// Decides whether the soft keyword `with` starts a `with` statement.
    fn line_has_block_colon(&self) -> bool {
        let mut depth = 0usize;
        for (i, kind) in self.rest_of_line() {
            match kind {
                L_PAREN | L_BRACK | L_BRACE => depth += 1,
                R_PAREN | R_BRACK | R_BRACE => depth = depth.saturating_sub(1),
                COLON if depth == 0 => {
                    let symbol = self
                        .tokens
                        .get(i + 1)
                        .is_some_and(|t| Self::is_name(t.kind));
                    if !symbol {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// True when the `(` at the cursor closes on this line and a `:` follows it.
    fn paren_group_then_colon(&self) -> bool {
        let mut depth = 0usize;
        let mut closed = false;
        for (_, kind) in self.rest_of_line() {
            if closed {
                return kind == COLON;
            }
            match kind {
                L_PAREN | L_BRACK | L_BRACE => depth += 1,
                R_PAREN | R_BRACK | R_BRACE => {
                    depth = depth.saturating_sub(1);
                    closed = depth == 0;
                }
                _ => {}
            }
        }
        false
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    fn expect(&mut self, kind: SyntaxKind, what: &str) -> bool {
        let found = self.eat(kind);
        if !found {
            self.error(format!("expected {what}"));
        }
        found
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(kind.into());
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    fn start_at(&mut self, cp: Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(cp, kind.into());
    }

    fn finish(&mut self) {
        self.builder.finish_node();
    }

    fn error(&mut self, message: impl Into<String>) {
        let at = self
            .nth_index(0)
            .map_or((self.src.len(), self.src.len()), |i| {
                (self.tokens[i].start, self.tokens[i].end)
            });
        self.errors
            .push(SyntaxError::new(message, range(at.0, at.1)));
    }

    /// Wraps the rest of the line, and any block indented under it, in an ERROR node.
    fn error_line(&mut self, message: impl Into<String>) {
        self.error(message);
        self.start(ERROR);
        self.skip_line_and_block();
        self.finish();
    }

    fn skip_line_and_block(&mut self) {
        while !self.at_any(&[NEWLINE, EOF, DEDENT]) {
            self.bump();
        }
        self.eat(NEWLINE);
        if self.at(INDENT) {
            let mut depth = 0usize;
            loop {
                match self.current() {
                    EOF => break,
                    INDENT => depth += 1,
                    DEDENT => depth -= 1,
                    _ => {}
                }
                self.bump();
                if depth == 0 {
                    break;
                }
            }
        }
    }

    /// Requires the end of a logical line; anything left over becomes an ERROR node.
    fn line_end(&mut self) {
        if self.eat(NEWLINE) || self.at_any(&[EOF, DEDENT]) {
            return;
        }
        if self.at(SEMICOLON) {
            self.error("one statement per line: start a new line instead of `;`");
            self.start(ERROR);
            self.bump();
            self.finish();
            if self.at_line_end() {
                self.eat(NEWLINE);
            } else {
                self.simple_stmt_line();
            }
            return;
        }
        self.error("expected end of line");
        self.start(ERROR);
        while !self.at_any(&[NEWLINE, EOF, DEDENT]) {
            self.bump();
        }
        self.finish();
        self.eat(NEWLINE);
    }

    // ------------------------------------------------------------ statements

    fn statement(&mut self) {
        let before = self.pos;
        let cp = self.checkpoint();
        let mut decorated = false;
        while self.at(AT) {
            self.decorator();
            decorated = true;
        }
        let word = if self.at(IDENT) {
            self.current_text()
        } else {
            ""
        };
        let next = self.nth(1);
        match self.current() {
            DEF_KW => self.fn_def(cp, FN_DEF),
            // Soft keywords (RFC 0001): a keyword only when a name (or `def`) follows.
            IDENT if word == "gene" && next == DEF_KW => self.fn_def(cp, GENE_DEF),
            IDENT if word == "gene" && next == IDENT => self.error_line(
                "only `gene def` is specified (RFC 0001); other `gene` forms need an RFC",
            ),
            IDENT if word == "module" && next == IDENT => self.module_def(cp),
            IDENT if word == "schema" && next == IDENT => self.schema_def(cp),
            IDENT if word == "fitness" && next == IDENT => self.fitness_def(cp),
            IDENT if RESERVED_DECLS.contains(&word) && next == IDENT => {
                self.error_line(format!(
                    "`{word}` declarations are not specified by an RFC yet (`{word}` is a soft keyword, so it still works as a name)"
                ));
            }
            IDENT
                if matches!(word, "function" | "fn" | "func")
                    && next == IDENT
                    && self.nth(2) == L_PAREN =>
            {
                self.error(format!(
                    "`{word}` is not Evolve: define functions with `def`"
                ));
                self.fn_def(cp, FN_DEF);
            }
            IDENT if word == "with" && !decorated && self.line_has_block_colon() => {
                self.with_stmt()
            }
            IDENT if word == "try" && next == COLON => {
                self.error("`try` is Python spelling: use `attempt:`");
                self.attempt_stmt();
            }
            IDENT if word == "except" && matches!(next, IDENT | COLON | L_PAREN) => self
                .error_line("`except` is Python spelling: use `learn`, after an `attempt:` block"),
            IDENT if word == "finally" && next == COLON => self.error_line(
                "`finally` is Python spelling: use `always:`, after an `attempt:` block",
            ),
            IF_KW => self.if_stmt(),
            FOR_KW => self.for_stmt(),
            WHILE_KW => self.while_stmt(),
            ATTEMPT_KW => self.attempt_stmt(),
            LEARN_KW | ALWAYS_KW => {
                self.error_line("`learn` and `always` must follow an `attempt` block")
            }
            ELIF_KW | ELSE_KW => self.error_line("`elif` and `else` must follow an `if` block"),
            INDENT => self.unexpected_indent(),
            _ if decorated => {
                self.error_line("decorators apply to `def`, `gene def`, `module`, or `schema`")
            }
            _ => self.simple_stmt_line(),
        }
        if self.pos == before {
            // Guarantee progress on any input.
            self.error("unexpected token");
            self.start(ERROR);
            self.bump();
            self.finish();
        }
    }

    fn unexpected_indent(&mut self) {
        self.error("unexpected indentation");
        self.start(ERROR);
        self.bump(); // INDENT
        while !self.at_any(&[DEDENT, EOF]) {
            self.statement();
        }
        self.eat(DEDENT);
        self.finish();
    }

    fn decorator(&mut self) {
        self.start(DECORATOR);
        self.bump(); // @
        self.expression(); // any expression (PEP 614)
        self.finish();
        self.expect(NEWLINE, "a new line after the decorator");
    }

    fn dotted_name(&mut self) {
        self.expect(IDENT, "a name");
        while self.at(DOT) && self.nth(1) == IDENT {
            self.bump();
            self.bump();
        }
    }

    fn fn_def(&mut self, cp: Checkpoint, kind: SyntaxKind) {
        self.start_at(cp, kind);
        if kind == GENE_DEF {
            self.bump_remap(GENE_KW);
        }
        self.bump(); // def
        self.expect(IDENT, "a function name");
        self.param_list();
        if self.at(ARROW) {
            self.start(RET_TYPE);
            self.bump();
            self.type_expr();
            self.finish();
        }
        self.expect(COLON, "`:`");
        let saved = (self.in_always, self.loop_in_always);
        (self.in_always, self.loop_in_always) = (false, false);
        self.block();
        (self.in_always, self.loop_in_always) = saved;
        self.finish();
    }

    fn param_list(&mut self) {
        self.start(PARAM_LIST);
        if self.expect(L_PAREN, "`(`") {
            let (mut star, mut double_star) = (false, false);
            while !self.at_any(&[R_PAREN, NEWLINE, EOF]) {
                let before = self.pos;
                self.start(PARAM);
                if double_star {
                    self.error("the `**` parameter must come last");
                }
                if self.at(STAR2) {
                    self.bump();
                    self.expect(IDENT, "a name for the keyword arguments");
                    double_star = true;
                } else if self.at(STAR) {
                    if star {
                        self.error("only one `*` parameter is allowed");
                    }
                    self.bump();
                    self.eat(IDENT); // bare `*`: the parameters after it are keyword-only
                    star = true;
                } else {
                    self.expect(IDENT, "a parameter name");
                }
                if self.eat(COLON) {
                    self.type_expr();
                }
                if self.eat(EQ) {
                    self.expression();
                }
                self.finish();
                if self.pos == before || !self.eat(COMMA) {
                    break;
                }
            }
            self.expect(R_PAREN, "`)`");
        }
        self.finish();
    }

    fn module_def(&mut self, cp: Checkpoint) {
        self.start_at(cp, MODULE_DEF);
        self.bump_remap(MODULE_KW);
        self.expect(IDENT, "a module name");
        if self.at(L_PAREN) {
            self.param_list();
        }
        self.expect(COLON, "`:`");
        self.block();
        self.finish();
    }

    fn schema_def(&mut self, cp: Checkpoint) {
        self.start_at(cp, SCHEMA_DEF);
        self.bump_remap(SCHEMA_KW);
        self.expect(IDENT, "a schema name");
        self.expect(COLON, "`:`");
        self.clause_block("a field or `invariant`", |p| {
            if p.at_contextual("invariant") {
                p.invariant_clause();
            } else if p.at(IDENT) && p.nth(1) == COLON {
                p.start(FIELD_DECL);
                p.bump();
                p.bump();
                p.type_expr();
                if p.eat(EQ) {
                    p.expression();
                }
                p.line_end();
                p.finish();
            } else {
                p.error_line("expected a field (`name: Type`) or `invariant`");
            }
        });
        self.finish();
    }

    fn fitness_def(&mut self, cp: Checkpoint) {
        self.start_at(cp, FITNESS_DEF);
        self.bump_remap(FITNESS_KW);
        self.expect(IDENT, "the name of the gene this fitness applies to");
        self.expect(COLON, "`:`");
        self.clause_block("a fitness clause", |p| {
            if FITNESS_INPUTS.iter().any(|w| p.at_contextual(w)) {
                p.start(FITNESS_CLAUSE);
                p.bump();
                p.expression_list();
                p.line_end();
                p.finish();
            } else if p.at_contextual("score") && p.nth(1) == EQ {
                p.start(SCORE_CLAUSE);
                p.bump();
                p.bump();
                p.expression();
                p.line_end();
                p.finish();
            } else if p.at_contextual("invariant") {
                p.invariant_clause();
            } else {
                p.error_line("expected a fitness clause: examples, holdout, benchmark, simulate, replay, score, or invariant");
            }
        });
        self.finish();
    }

    fn invariant_clause(&mut self) {
        self.start(INVARIANT_CLAUSE);
        self.bump();
        self.expression();
        self.line_end();
        self.finish();
    }

    /// A `NEWLINE INDENT clause+ DEDENT` body made of special clauses.
    fn clause_block(&mut self, what: &str, mut clause: impl FnMut(&mut Self)) {
        self.start(BLOCK);
        if self.expect(NEWLINE, "a new line") && self.eat(INDENT) {
            while !self.at_any(&[DEDENT, EOF]) {
                let before = self.pos;
                clause(self);
                if self.pos == before {
                    self.error_line(format!("expected {what}"));
                }
            }
            self.eat(DEDENT);
        } else {
            self.error(format!("expected an indented block of {what}"));
        }
        self.finish();
    }

    fn block(&mut self) {
        self.start(BLOCK);
        if self.eat(NEWLINE) {
            if self.eat(INDENT) {
                while !self.at_any(&[DEDENT, EOF]) {
                    self.statement();
                }
                self.eat(DEDENT);
            } else {
                self.error("expected an indented block");
            }
        } else {
            self.simple_stmt_line();
        }
        self.finish();
    }

    fn if_stmt(&mut self) {
        self.start(IF_STMT);
        self.bump();
        self.expression();
        self.expect(COLON, "`:`");
        self.block();
        loop {
            if self.at(ELSE_KW) && self.nth(1) == IF_KW {
                self.start(ELIF_CLAUSE);
                self.error("write `elif`, not `else if`");
                self.bump();
                self.bump();
            } else if self.at(ELIF_KW) {
                self.start(ELIF_CLAUSE);
                self.bump();
            } else {
                break;
            }
            self.expression();
            self.expect(COLON, "`:`");
            self.block();
            self.finish();
        }
        if self.at(ELSE_KW) {
            self.start(ELSE_CLAUSE);
            self.bump();
            self.expect(COLON, "`:`");
            self.block();
            self.finish();
        }
        self.finish();
    }

    fn for_stmt(&mut self) {
        self.start(FOR_STMT);
        self.bump();
        self.target_list();
        self.expect(IN_KW, "`in`");
        self.expression_list();
        self.expect(COLON, "`:`");
        self.loop_body();
        self.finish();
    }

    /// Targets of `for` and comprehensions: `a`, `a, b`, `(a, b)`, `first, *rest`.
    fn target_list(&mut self) {
        self.start(TARGET_LIST);
        loop {
            if self.at(STAR) {
                self.start(STARRED_EXPR);
                self.bump();
                self.postfix_expr();
                self.finish();
            } else {
                self.postfix_expr();
            }
            if !self.eat(COMMA) || self.at(IN_KW) {
                break;
            }
        }
        self.finish();
    }

    fn loop_body(&mut self) {
        let saved = self.loop_in_always;
        self.loop_in_always = self.in_always;
        self.block();
        self.loop_in_always = saved;
        if self.at(ELSE_KW) && self.nth(1) == COLON {
            self.error_line("loops have no `else` clause in Evolve; use a flag or `any(...)`");
        }
    }

    fn while_stmt(&mut self) {
        self.start(WHILE_STMT);
        self.bump();
        self.expression();
        self.expect(COLON, "`:`");
        self.loop_body();
        self.finish();
    }

    fn attempt_stmt(&mut self) {
        self.start(ATTEMPT_STMT);
        self.bump(); // `attempt` (or `try`, already reported)
        self.expect(COLON, "`:`");
        self.block();
        let at_learn = |p: &Self| {
            p.at(LEARN_KW)
                || (p.at_contextual("except") && matches!(p.nth(1), IDENT | COLON | L_PAREN))
        };
        if !at_learn(self) {
            self.error("`attempt` needs at least one `learn` clause");
        }
        while at_learn(self) {
            self.start(LEARN_CLAUSE);
            if self.at(IDENT) {
                self.error("`except` is Python spelling: use `learn`");
            }
            self.bump();
            if self.at(IDENT) && self.nth(1) == COLON {
                self.bump(); // `learn err:` catches everything
            } else {
                self.type_expr();
                self.expect(AS_KW, "`as`");
                self.expect(IDENT, "a name for the error");
            }
            self.expect(COLON, "`:`");
            self.block();
            self.finish();
        }
        let finally = self.at_contextual("finally") && self.nth(1) == COLON;
        if self.at(ALWAYS_KW) || finally {
            self.start(ALWAYS_CLAUSE);
            if finally {
                self.error("`finally` is Python spelling: use `always:`");
            }
            self.bump();
            self.expect(COLON, "`:`");
            let saved = (self.in_always, self.loop_in_always);
            (self.in_always, self.loop_in_always) = (true, false);
            self.block();
            (self.in_always, self.loop_in_always) = saved;
            self.finish();
        }
        self.finish();
    }

    fn with_stmt(&mut self) {
        self.start(WITH_STMT);
        self.bump_remap(WITH_KW);
        if self.at(L_PAREN) && self.paren_group_then_colon() {
            self.bump();
            while !self.at_any(&[R_PAREN, NEWLINE, EOF]) {
                let before = self.pos;
                self.with_item();
                if self.pos == before || !self.eat(COMMA) {
                    break;
                }
            }
            self.expect(R_PAREN, "`,` or `)`");
        } else {
            self.with_item();
            while self.eat(COMMA) {
                self.with_item();
            }
        }
        self.expect(COLON, "`:`");
        self.block();
        self.finish();
    }

    fn with_item(&mut self) {
        self.start(WITH_ITEM);
        self.expression();
        if self.eat(AS_KW) {
            self.postfix_expr(); // one target: `as f` or `as (a, b)`
        }
        self.finish();
    }

    fn simple_stmt_line(&mut self) {
        self.simple_stmt();
        self.line_end();
    }

    fn simple_stmt(&mut self) {
        match self.current() {
            RETURN_KW => {
                self.start(RETURN_STMT);
                if self.in_always {
                    self.error("`return` inside `always` would discard the failure in flight");
                }
                self.bump();
                if !self.at_line_end() {
                    self.expression_list();
                }
                self.finish();
            }
            RAISE_KW => {
                self.start(RAISE_STMT);
                self.bump();
                if !self.at_line_end() && !self.at(SEMICOLON) {
                    self.expression();
                    if self.eat(FROM_KW) {
                        self.expression();
                    }
                }
                self.finish();
            }
            ASSERT_KW => {
                self.start(ASSERT_STMT);
                self.bump();
                self.expression();
                if self.eat(COMMA) {
                    self.expression();
                }
                self.finish();
            }
            REMEMBER_KW => {
                self.start(REMEMBER_STMT);
                self.bump();
                self.expression();
                self.finish();
            }
            IMPORT_KW => {
                self.start(IMPORT_STMT);
                self.bump();
                loop {
                    self.start(IMPORT_ITEM);
                    self.dotted_name();
                    if self.eat(AS_KW) {
                        self.expect(IDENT, "a name");
                    }
                    self.finish();
                    if !self.eat(COMMA) {
                        break;
                    }
                }
                self.finish();
            }
            FROM_KW => {
                self.start(IMPORT_STMT);
                self.bump();
                self.dotted_name();
                self.expect(IMPORT_KW, "`import`");
                if self.at(STAR) {
                    self.error(
                        "name what you import: `from m import *` hides where names come from",
                    );
                    self.start(ERROR);
                    self.bump();
                    self.finish();
                } else {
                    let paren = self.eat(L_PAREN);
                    loop {
                        if paren && self.at(R_PAREN) {
                            break;
                        }
                        self.start(IMPORT_ITEM);
                        self.expect(IDENT, "a name to import");
                        if self.eat(AS_KW) {
                            self.expect(IDENT, "a name");
                        }
                        self.finish();
                        if !self.eat(COMMA) {
                            break;
                        }
                    }
                    if paren {
                        self.expect(R_PAREN, "`,` or `)`");
                    }
                }
                self.finish();
            }
            PASS_KW | BREAK_KW | CONTINUE_KW => {
                let kind = match self.current() {
                    PASS_KW => PASS_STMT,
                    BREAK_KW => BREAK_STMT,
                    _ => CONTINUE_STMT,
                };
                self.start(kind);
                if kind != PASS_STMT && self.in_always && !self.loop_in_always {
                    let word = self.current_text();
                    self.error(format!(
                        "`{word}` inside `always` would discard the failure in flight"
                    ));
                }
                self.bump();
                self.finish();
            }
            _ => self.expr_or_assign(),
        }
    }

    fn expr_or_assign(&mut self) {
        let cp = self.checkpoint();
        self.expression_list();
        match self.current() {
            EQ => {
                self.start_at(cp, ASSIGN_STMT);
                while self.eat(EQ) {
                    self.expression_list();
                }
            }
            PLUS_EQ | MINUS_EQ | STAR_EQ | SLASH_EQ | SLASH2_EQ | PERCENT_EQ | CARET_EQ => {
                self.start_at(cp, AUG_ASSIGN_STMT);
                self.bump();
                self.expression();
            }
            COLON => {
                self.start_at(cp, ASSIGN_STMT);
                self.bump();
                self.type_expr();
                if self.eat(EQ) {
                    self.expression_list();
                }
            }
            _ => self.start_at(cp, EXPR_STMT),
        }
        self.finish();
    }

    // ------------------------------------------------------------ types

    fn type_expr(&mut self) {
        let cp = self.checkpoint();
        self.type_atom();
        if self.at(PIPE) {
            self.start_at(cp, UNION_TYPE);
            while self.eat(PIPE) {
                self.type_atom();
            }
            self.finish();
        }
    }

    fn type_atom(&mut self) {
        match self.current() {
            NONE_KW => {
                self.start(TYPE_REF);
                self.bump();
                self.finish();
            }
            L_PAREN => {
                self.bump();
                self.type_expr();
                self.expect(R_PAREN, "`)`");
            }
            IDENT => {
                self.start(TYPE_REF);
                self.dotted_name();
                if self.at(L_BRACK) {
                    self.start(TYPE_ARGS);
                    self.bump();
                    self.type_expr();
                    while self.eat(COMMA) {
                        self.type_expr();
                    }
                    self.expect(R_BRACK, "`]`");
                    self.finish();
                }
                self.finish();
            }
            _ => self.error("expected a type"),
        }
    }

    // ------------------------------------------------------------ expressions

    fn expression_list(&mut self) {
        let cp = self.checkpoint();
        self.star_or_expression();
        if self.at(COMMA) {
            self.start_at(cp, TUPLE_EXPR);
            while self.eat(COMMA) {
                if self.at_line_end() || self.at_any(&[EQ, R_PAREN, R_BRACK, R_BRACE]) {
                    break;
                }
                self.star_or_expression();
            }
            self.finish();
        }
    }

    /// An expression, or `*expr` where unpacking is allowed (displays, targets, calls).
    fn star_or_expression(&mut self) {
        if self.at(STAR) {
            self.start(STARRED_EXPR);
            self.bump();
            self.pipe_expr();
            self.finish();
        } else {
            self.expression();
        }
    }

    pub(crate) fn expression(&mut self) {
        if self.at(IDENT) && self.nth(1) == ARROW {
            self.start(LAMBDA);
            self.bump();
            self.bump();
            self.expression();
            self.finish();
            return;
        }
        let cp = self.checkpoint();
        self.pipe_expr();
        if self.at(ARROW) {
            self.start_at(cp, LAMBDA);
            self.bump();
            self.expression();
            self.finish();
        } else if self.at(IF_KW) {
            // `x if c else y` binds looser than `|>` (RFC 0001).
            self.start_at(cp, CONDITIONAL_EXPR);
            self.bump();
            self.pipe_expr();
            self.expect(
                ELSE_KW,
                "`else`: a conditional expression is `x if c else y`",
            );
            self.expression();
            self.finish();
        }
    }

    fn pipe_expr(&mut self) {
        let cp = self.checkpoint();
        self.binary(Self::or_expr, &[PIPE_GT], cp);
    }

    /// Hints for operators from other languages, parsed anyway for recovery.
    fn operator_hint(&mut self) {
        let message = match self.current() {
            AMP2 => "use `and`, not `&&`",
            PIPE2 => "use `or`, not `||`",
            AT => {
                "`@` is not an operator in Evolve: `*` is matrix multiply for 2-d arrays, `.*` is element-wise"
            }
            STAR2 => "`**` is not Evolve syntax; use `^` for powers",
            _ => return,
        };
        self.error(message);
    }

    /// Left-associative binary level: `next (op next)*`.
    fn binary(&mut self, next: fn(&mut Self), ops: &[SyntaxKind], cp: Checkpoint) {
        next(self);
        while self.at_any(ops) {
            self.operator_hint();
            self.start_at(cp, BIN_EXPR);
            self.bump();
            next(self);
            self.finish();
        }
    }

    fn or_expr(&mut self) {
        let cp = self.checkpoint();
        self.binary(Self::and_expr, &[OR_KW, PIPE2], cp);
    }

    fn and_expr(&mut self) {
        let cp = self.checkpoint();
        self.binary(Self::not_expr, &[AND_KW, AMP2], cp);
    }

    fn not_expr(&mut self) {
        if self.at_any(&[NOT_KW, BANG]) {
            if self.at(BANG) {
                self.error("use `not x`, not `!x`");
            }
            self.start(PREFIX_EXPR);
            self.bump();
            self.not_expr();
            self.finish();
        } else {
            self.comparison();
        }
    }

    /// Comparison chains keep one direction; `!=`, `in`, and `is` never chain (RFC 0001).
    fn comparison(&mut self) {
        let cp = self.checkpoint();
        self.range_expr();
        let (mut up, mut down, mut other, mut count, mut reported) =
            (false, false, false, 0, false);
        loop {
            let two_word = (self.at(NOT_KW) && self.nth(1) == IN_KW)
                || (self.at(IS_KW) && self.nth(1) == NOT_KW);
            let op = self.current();
            if !two_word && !matches!(op, EQ2 | NEQ | LT | LTEQ | GT | GTEQ | IN_KW | IS_KW) {
                break;
            }
            count += 1;
            match op {
                LT | LTEQ => up = true,
                GT | GTEQ => down = true,
                EQ2 => {}
                _ => other = true,
            }
            if count >= 2 && !reported && (other || (up && down)) {
                reported = true;
                self.error(if other {
                    "`!=`, `in`, and `is` don't chain: add parentheses, or split the chain with `and`"
                } else {
                    "this comparison chain goes both ways: split it with `and`, e.g. `x < y and z < y`"
                });
            }
            self.start_at(cp, BIN_EXPR);
            self.bump();
            if two_word {
                self.bump();
            }
            let literal = self.at_any(&[NUMBER, STRING, FSTRING])
                || (self.at(MINUS) && self.nth(1) == NUMBER);
            if op == IS_KW && literal {
                self.error("`is` compares identity, which literals don't have: use `==`");
            }
            self.range_expr();
            self.finish();
        }
    }

    fn range_expr(&mut self) {
        let cp = self.checkpoint();
        self.pm_expr();
        if self.at_any(&[DOT2, DOT2EQ]) {
            self.start_at(cp, BIN_EXPR);
            self.bump();
            self.pm_expr();
            self.finish();
        }
    }

    fn pm_expr(&mut self) {
        let cp = self.checkpoint();
        self.binary(Self::arith, &[PLUS_MINUS], cp);
    }

    fn arith(&mut self) {
        let cp = self.checkpoint();
        self.binary(Self::term, &[PLUS, MINUS, DOT_PLUS, DOT_MINUS], cp);
    }

    fn term(&mut self) {
        let cp = self.checkpoint();
        self.binary(
            Self::unary,
            &[
                STAR,
                SLASH,
                SLASH2,
                BACKSLASH,
                PERCENT,
                DOT_STAR,
                DOT_SLASH,
                DOT_BACKSLASH,
                AT,
            ],
            cp,
        );
    }

    fn unary(&mut self) {
        if self.at_any(&[MINUS, PLUS]) {
            self.start(PREFIX_EXPR);
            self.bump();
            self.unary();
            self.finish();
        } else {
            self.power();
        }
    }

    fn power(&mut self) {
        let cp = self.checkpoint();
        self.postfix_expr();
        if self.at_any(&[CARET, DOT_CARET, STAR2]) {
            self.operator_hint();
            self.start_at(cp, BIN_EXPR);
            self.bump();
            self.unary(); // right-associative: 2 ^ 3 ^ 2 == 2 ^ (3 ^ 2)
            self.finish();
        }
    }

    fn postfix_expr(&mut self) {
        let cp = self.checkpoint();
        let first = self.atom();
        if first == Some(NUMBER) && !self.no_units && self.at(IDENT) {
            self.start_at(cp, UNIT_EXPR);
            self.unit_suffix();
            self.finish();
            return;
        }
        loop {
            match self.current() {
                L_PAREN => {
                    self.start_at(cp, CALL_EXPR);
                    self.arg_list();
                }
                DOT if self.nth(1) == L_PAREN => {
                    self.start_at(cp, BCAST_CALL_EXPR);
                    self.bump();
                    self.arg_list();
                }
                DOT if Self::is_name(self.nth(1)) => {
                    self.start_at(cp, FIELD_EXPR);
                    self.bump();
                    self.bump();
                }
                L_BRACK => {
                    self.start_at(cp, INDEX_EXPR);
                    self.slice_list();
                }
                _ => break,
            }
            self.finish();
        }
    }

    fn unit_suffix(&mut self) {
        self.start(UNIT_SUFFIX);
        self.unit_atom();
        while self.at_any(&[STAR, SLASH]) && self.nth(1) == IDENT {
            self.bump();
            self.unit_atom();
        }
        self.finish();
    }

    fn unit_atom(&mut self) {
        self.bump(); // unit name
        if self.at(SUPERSCRIPT) {
            self.bump();
        } else if self.at(CARET) && self.nth(1) == NUMBER {
            self.bump();
            self.bump();
        }
    }

    /// `(args)`: positional and `*splat` before `name=` and `**splat`; a generator
    /// expression may be the only argument without its own parentheses.
    fn arg_list(&mut self) {
        self.start(ARG_LIST);
        self.bump(); // (
        let (mut count, mut keywords) = (0, false);
        while !self.at_any(&[R_PAREN, NEWLINE, EOF]) {
            let before = self.pos;
            if Self::is_name(self.current()) && self.nth(1) == EQ {
                self.start(NAMED_ARG);
                self.bump();
                self.bump();
                self.expression();
                keywords = true;
            } else if self.at(STAR2) {
                self.start(ARG);
                self.start(STARRED_EXPR);
                self.bump();
                self.pipe_expr();
                self.finish();
                keywords = true;
            } else {
                self.start(ARG);
                if keywords {
                    self.error(
                        "positional arguments and `*` unpacking come before keyword arguments",
                    );
                }
                let cp = self.checkpoint();
                self.star_or_expression();
                if self.at(FOR_KW) {
                    self.start_at(cp, GENERATOR_EXPR);
                    self.comp_clauses();
                    self.finish();
                    if count > 0 || (self.at(COMMA) && self.nth(1) != R_PAREN) {
                        self.error(
                            "a generator expression must be the only argument; wrap it in parentheses",
                        );
                    }
                }
            }
            self.finish();
            count += 1;
            if self.pos == before || !self.eat(COMMA) {
                break;
            }
        }
        self.expect(R_PAREN, "`,` or `)`");
        self.finish();
    }

    /// `for targets in iterable (if condition)*`, repeated. The iterable and the
    /// condition are pipe-level, so a conditional expression there needs parentheses.
    fn comp_clauses(&mut self) {
        while self.at(FOR_KW) {
            self.start(COMP_FOR);
            self.bump();
            self.target_list();
            self.expect(IN_KW, "`in`");
            self.pipe_expr();
            self.finish();
            while self.at(IF_KW) {
                self.start(COMP_IF);
                self.bump();
                self.pipe_expr();
                self.finish();
            }
        }
    }

    fn slice_list(&mut self) {
        self.start(SLICE_LIST);
        self.bump(); // [
        loop {
            self.slice();
            if !self.eat(COMMA) || self.at(R_BRACK) {
                break;
            }
        }
        self.expect(R_BRACK, "`,` or `]`");
        self.finish();
    }

    /// `expr`, or a Python slice `[start]:[stop][:step]`. `:name` with no space is a
    /// symbol, not a slice (RFC 0001), so `df[:, :period]` selects a column.
    fn slice(&mut self) {
        let cp = self.checkpoint();
        let starts_with_symbol =
            self.at(COLON) && self.next_is_adjacent() && Self::is_name(self.nth(1));
        if !self.at(COLON) || starts_with_symbol {
            self.star_or_expression();
        }
        if self.at(COLON) {
            self.start_at(cp, SLICE);
            self.bump();
            if !self.at_any(&[COLON, COMMA, R_BRACK]) {
                self.expression();
            }
            if self.eat(COLON) && !self.at_any(&[COMMA, R_BRACK]) {
                self.expression();
            }
            self.finish();
        }
    }

    /// Parses one atom and returns the kind of its first token.
    fn atom(&mut self) -> Option<SyntaxKind> {
        let kind = self.current();
        match kind {
            IDENT if self.next_is_adjacent() && matches!(self.nth(1), STRING) => {
                self.start(STRING_MACRO);
                self.bump();
                self.bump();
                self.finish();
                self.adjacent_strings();
            }
            IDENT => {
                let hint = match self.current_text() {
                    "True" => Some("use `true`"),
                    "False" => Some("use `false`"),
                    "None" => Some("use `none`"),
                    "lambda" => Some("lambdas are written `x -> expr`"),
                    _ => None,
                };
                if let Some(hint) = hint {
                    let word = self.current_text();
                    self.error(format!("`{word}` is Python spelling: {hint}"));
                }
                self.start(NAME_REF);
                self.bump();
                self.finish();
            }
            STRING | FSTRING => {
                self.start(LITERAL);
                self.bump();
                self.finish();
                self.adjacent_strings();
            }
            NUMBER | TRUE_KW | FALSE_KW | NONE_KW | DOT3 => {
                self.start(LITERAL);
                self.bump();
                self.finish();
            }
            COLON if self.next_is_adjacent() && Self::is_name(self.nth(1)) => {
                self.start(SYMBOL);
                self.bump();
                self.bump();
                self.finish();
            }
            L_PAREN => self.paren_or_tuple(),
            L_BRACK => self.list_display(),
            L_BRACE => self.dict_or_set(),
            TILDE => self.evolvable(),
            _ => {
                self.error("expected an expression");
                let sync = [
                    NEWLINE, EOF, DEDENT, INDENT, R_PAREN, R_BRACK, R_BRACE, COMMA, COLON, EQ,
                ];
                if !self.at_any(&sync) {
                    self.start(ERROR);
                    self.bump();
                    self.finish();
                }
                return None;
            }
        }
        Some(kind)
    }

    /// `"a" "b"` is an error: implicit concatenation hides missing commas (RFC 0001).
    fn adjacent_strings(&mut self) {
        while self.at_any(&[STRING, FSTRING])
            || (self.at(IDENT) && self.next_is_adjacent() && self.nth(1) == STRING)
        {
            self.error(
                "two string literals in a row: add the missing comma, or join them with `+`",
            );
            self.start(ERROR);
            if self.at(IDENT) {
                self.bump();
            }
            self.bump();
            self.finish();
        }
    }

    fn paren_or_tuple(&mut self) {
        let cp = self.checkpoint();
        self.bump(); // (
        let mut kind = TUPLE_EXPR;
        if !self.at(R_PAREN) {
            self.star_or_expression();
            if self.at(FOR_KW) {
                self.comp_clauses();
                kind = GENERATOR_EXPR;
            } else if self.at(COMMA) {
                while self.eat(COMMA) && !self.at(R_PAREN) {
                    self.star_or_expression();
                }
            } else {
                kind = PAREN_EXPR;
            }
        }
        self.expect(R_PAREN, "`)`");
        self.start_at(cp, kind);
        self.finish();
    }

    fn list_display(&mut self) {
        let cp = self.checkpoint();
        self.bump(); // [
        let mut kind = LIST_EXPR;
        let mut first = true;
        while !self.at_any(&[R_BRACK, NEWLINE, EOF]) {
            let before = self.pos;
            self.star_or_expression();
            if first && self.at(FOR_KW) {
                self.comp_clauses();
                kind = LIST_COMP;
                break;
            }
            first = false;
            if self.pos == before || !self.eat(COMMA) {
                break;
            }
        }
        self.expect(R_BRACK, "`,` or `]`");
        self.start_at(cp, kind);
        self.finish();
    }

    fn dict_or_set(&mut self) {
        let cp = self.checkpoint();
        self.bump(); // {
        let mut kind = DICT_EXPR; // `{}` is an empty dict
        let mut first = true;
        let mut mixed = false;
        while !self.at_any(&[R_BRACE, NEWLINE, EOF]) {
            let before = self.pos;
            let entry_range = self
                .nth_index(0)
                .map_or(range(self.src.len(), self.src.len()), |i| {
                    range(self.tokens[i].start, self.tokens[i].end)
                });
            let errors_before = self.errors.len();
            let mut entry_is_pair = true;
            if self.at(STAR2) {
                self.start(STARRED_EXPR);
                self.bump();
                self.pipe_expr();
                self.finish();
            } else {
                let entry = self.checkpoint();
                self.star_or_expression();
                if self.at(COLON) {
                    self.start_at(entry, DICT_ENTRY);
                    self.bump();
                    self.expression();
                    self.finish();
                } else {
                    entry_is_pair = false;
                }
            }
            if first {
                kind = if entry_is_pair { DICT_EXPR } else { SET_EXPR };
                if self.at(FOR_KW) {
                    self.comp_clauses();
                    kind = if entry_is_pair { DICT_COMP } else { SET_COMP };
                    break;
                }
            } else if entry_is_pair != (kind == DICT_EXPR)
                && !mixed
                && self.errors.len() == errors_before
            {
                // The first entry decides between dict and set (spec/grammar.ebnf `brace`).
                // An entry that already has an error of its own gets no second one.
                mixed = true;
                let message = if kind == DICT_EXPR {
                    "a dict entry is `key: value` or `**mapping`: this entry has no value"
                } else {
                    "a set entry is a single value: `key: value` and `**mapping` entries belong in a dict"
                };
                self.errors.push(SyntaxError::new(message, entry_range));
            }
            first = false;
            if self.pos == before || !self.eat(COMMA) {
                break;
            }
        }
        self.expect(R_BRACE, "`,` or `}`");
        self.start_at(cp, kind);
        self.finish();
    }

    /// `~value [in lo..hi [scale]]` (RFC 0001, RFC 0003).
    fn evolvable(&mut self) {
        self.start(EVOLVABLE);
        self.bump(); // ~
        self.arith();
        if self.eat(IN_KW) {
            let saved = self.no_units;
            self.no_units = true;
            self.range_expr();
            self.no_units = saved;
            if self.at(IDENT) {
                self.bump(); // scale hint, e.g. `log`
            }
        }
        self.finish();
    }
}
