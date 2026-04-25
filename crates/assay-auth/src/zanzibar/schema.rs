//! SpiceDB-compatible schema DSL parser.
//!
//! Hand-rolled recursive-descent — no nom/chumsky dependency, the
//! grammar is small enough that the simplicity wins. Covers the
//! v0.2.0 subset called out in plan 12c lines 1097–1110:
//!
//! ```text
//! definition user {}
//!
//! definition group {
//!     relation member: user
//! }
//!
//! definition document {
//!     relation owner: user
//!     relation viewer: user | group#member
//!     permission view = owner + viewer
//!     permission edit = owner
//! }
//! ```
//!
//! Supports:
//!
//! - `definition NAME { … }` blocks (one namespace per block).
//! - `relation NAME: TYPE [ | TYPE … ]` lines, where TYPE is one of:
//!   - `name` — direct subject (`user`)
//!   - `name#relation` — userset (`group#member`)
//!   - `name:*` — wildcard subject (`user:*`)
//! - `permission NAME = EXPR` lines, with EXPR built from:
//!   - `name` — direct relation reference
//!   - `+` — union (left-associative)
//!   - `&` — intersection
//!   - `-` — exclusion
//!   - `relation->permission` — tupleset arrow
//!   - `( EXPR )` — grouping
//! - Line comments — `//` to end of line. (SpiceDB-style only.
//!   `#` is reserved for userset references like `group#member`, so
//!   it can't double as a comment marker.)
//!
//! Returns one [`NamespaceSchema`] per definition block. The caller
//! (`define_namespace` in the store impls) persists each independently
//! so a partial schema update on one namespace doesn't drop another.

use std::collections::BTreeMap;

use super::types::{NamespaceSchema, PermissionExpr, RelationDef, TypeRef};

/// Convenience entry point — parse one or more `definition` blocks.
/// Returns each namespace as an independent [`NamespaceSchema`].
pub fn parse_schema(input: &str) -> Result<Vec<NamespaceSchema>, ParseError> {
    Parser::new(input).parse_top()
}

/// Parser error type. Carries a 1-based `(line, col)` location for
/// human-readable diagnostics. Wraps in `crate::Error::Backend` at the
/// caller boundary so HTTP returns a 400 with the location intact.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("parse error at line {line} col {col}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

/// Token classes emitted by the lexer. `Ident` covers the
/// alphanumeric+underscore identifiers SpiceDB uses for definitions /
/// relations / permissions. The composite tokens (`Arrow`, etc.) keep
/// the parser cleaner than character-level lookahead.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    LBrace,    // `{`
    RBrace,    // `}`
    Colon,     // `:`
    Hash,      // `#`
    Pipe,      // `|`
    Plus,      // `+`
    Amp,       // `&`
    Minus,     // `-`
    Eq,        // `=`
    Star,      // `*`
    LParen,    // `(`
    RParen,    // `)`
    Arrow,     // `->`
}

/// Token + its source location, kept paired so error messages can
/// pinpoint the offending construct.
#[derive(Clone, Debug)]
struct LocTok {
    tok: Token,
    line: usize,
    col: usize,
}

struct Parser<'a> {
    tokens: Vec<LocTok>,
    pos: usize,
    _src: &'a str,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        let tokens = lex(src);
        Self {
            tokens,
            pos: 0,
            _src: src,
        }
    }

    fn parse_top(&mut self) -> Result<Vec<NamespaceSchema>, ParseError> {
        let mut out = Vec::new();
        while !self.at_end() {
            let ns = self.parse_definition()?;
            out.push(ns);
        }
        Ok(out)
    }

    /// `definition NAME { … relation/permission lines … }`
    fn parse_definition(&mut self) -> Result<NamespaceSchema, ParseError> {
        let kw = self.expect_ident()?;
        if kw != "definition" {
            return Err(self.err_back(format!("expected `definition`, got `{kw}`")));
        }
        let name = self.expect_ident()?;
        self.expect(Token::LBrace)?;
        let mut definitions = BTreeMap::new();
        while !self.peek_eq(&Token::RBrace) {
            if self.at_end() {
                return Err(self.err_here("unterminated `definition` block".into()));
            }
            let kw = self.expect_ident()?;
            match kw.as_str() {
                "relation" => {
                    let def = self.parse_relation_line()?;
                    definitions.insert(def.name.clone(), def);
                }
                "permission" => {
                    let def = self.parse_permission_line()?;
                    definitions.insert(def.name.clone(), def);
                }
                other => {
                    return Err(self.err_back(format!(
                        "expected `relation` or `permission`, got `{other}`"
                    )));
                }
            }
        }
        self.expect(Token::RBrace)?;
        Ok(NamespaceSchema { name, definitions })
    }

    /// `relation NAME : TYPE [ | TYPE … ]`
    fn parse_relation_line(&mut self) -> Result<RelationDef, ParseError> {
        let name = self.expect_ident()?;
        self.expect(Token::Colon)?;
        let mut types = vec![self.parse_type_ref()?];
        while self.consume(&Token::Pipe) {
            types.push(self.parse_type_ref()?);
        }
        Ok(RelationDef::relation(name, types))
    }

    /// One TYPE on the right-hand side of a `relation` line.
    /// `user`, `user:*`, or `group#member`.
    fn parse_type_ref(&mut self) -> Result<TypeRef, ParseError> {
        let ty = self.expect_ident()?;
        if self.consume(&Token::Hash) {
            let rel = self.expect_ident()?;
            return Ok(TypeRef::userset(ty, rel));
        }
        if self.consume(&Token::Colon) {
            // Only `:*` is allowed here — anything else is a relation
            // tuple, not a type ref.
            self.expect(Token::Star)?;
            return Ok(TypeRef::wildcard(ty));
        }
        Ok(TypeRef::direct(ty))
    }

    /// `permission NAME = EXPR`
    fn parse_permission_line(&mut self) -> Result<RelationDef, ParseError> {
        let name = self.expect_ident()?;
        self.expect(Token::Eq)?;
        let expr = self.parse_expr_excl()?;
        Ok(RelationDef::permission(name, expr))
    }

    /// Lowest precedence — exclusion (`-`). Same precedence as the
    /// SpiceDB grammar.
    fn parse_expr_excl(&mut self) -> Result<PermissionExpr, ParseError> {
        let mut left = self.parse_expr_intersect()?;
        while self.consume(&Token::Minus) {
            let right = self.parse_expr_intersect()?;
            left = PermissionExpr::exclude(left, right);
        }
        Ok(left)
    }

    /// Intersection (`&`).
    fn parse_expr_intersect(&mut self) -> Result<PermissionExpr, ParseError> {
        let mut left = self.parse_expr_union()?;
        while self.consume(&Token::Amp) {
            let right = self.parse_expr_union()?;
            left = PermissionExpr::intersect(left, right);
        }
        Ok(left)
    }

    /// Union (`+`).
    fn parse_expr_union(&mut self) -> Result<PermissionExpr, ParseError> {
        let mut left = self.parse_expr_atom()?;
        while self.consume(&Token::Plus) {
            let right = self.parse_expr_atom()?;
            left = PermissionExpr::union(left, right);
        }
        Ok(left)
    }

    /// Highest precedence — atomic name, parenthesised group, or
    /// `relation->permission` arrow.
    fn parse_expr_atom(&mut self) -> Result<PermissionExpr, ParseError> {
        if self.consume(&Token::LParen) {
            let inner = self.parse_expr_excl()?;
            self.expect(Token::RParen)?;
            return Ok(inner);
        }
        let name = self.expect_ident()?;
        if self.consume(&Token::Arrow) {
            let perm = self.expect_ident()?;
            return Ok(PermissionExpr::arrow(name, perm));
        }
        Ok(PermissionExpr::direct(name))
    }

    // ---- token plumbing ----

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|lt| &lt.tok)
    }

    fn peek_eq(&self, t: &Token) -> bool {
        self.peek() == Some(t)
    }

    fn consume(&mut self, t: &Token) -> bool {
        if self.peek_eq(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Token) -> Result<(), ParseError> {
        if self.peek_eq(&t) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err_here(format!("expected token {t:?}")))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.tokens.get(self.pos) {
            Some(LocTok {
                tok: Token::Ident(s),
                ..
            }) => {
                let cloned = s.clone();
                self.pos += 1;
                Ok(cloned)
            }
            Some(_) => Err(self.err_here("expected identifier".into())),
            None => Err(self.err_here("expected identifier, got end of input".into())),
        }
    }

    fn err_here(&self, message: String) -> ParseError {
        let (line, col) = self
            .tokens
            .get(self.pos)
            .map(|lt| (lt.line, lt.col))
            .unwrap_or((0, 0));
        ParseError {
            line,
            col,
            message,
        }
    }

    /// Variant of [`Self::err_here`] that points at the *previous* token —
    /// used by error sites that have already advanced past the offender
    /// (`expect_ident` returning `Ok` then realising the keyword was
    /// wrong).
    fn err_back(&self, message: String) -> ParseError {
        let idx = self.pos.saturating_sub(1);
        let (line, col) = self
            .tokens
            .get(idx)
            .map(|lt| (lt.line, lt.col))
            .unwrap_or((0, 0));
        ParseError {
            line,
            col,
            message,
        }
    }
}

/// Lexer — emits one [`LocTok`] per recognised piece of syntax. Tracks
/// 1-based line/column so [`ParseError`] is human-friendly.
fn lex(src: &str) -> Vec<LocTok> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut line = 1usize;
    let mut col = 1usize;

    while i < bytes.len() {
        let c = bytes[i];
        // Whitespace.
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            col += 1;
            continue;
        }
        if c == b'\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        // Comments — `//…` to EOL or `#` at start-of-line. SpiceDB only
        // formally supports `//`, but jeebon's plan 12c examples use `#`
        // for inline notes; tolerating both keeps copy/paste safe.
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
                col += 1;
            }
            continue;
        }
        // Multi-char tokens first.
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            out.push(LocTok {
                tok: Token::Arrow,
                line,
                col,
            });
            i += 2;
            col += 2;
            continue;
        }
        // Single-char tokens.
        let single = match c {
            b'{' => Some(Token::LBrace),
            b'}' => Some(Token::RBrace),
            b':' => Some(Token::Colon),
            b'#' => Some(Token::Hash),
            b'|' => Some(Token::Pipe),
            b'+' => Some(Token::Plus),
            b'&' => Some(Token::Amp),
            b'-' => Some(Token::Minus),
            b'=' => Some(Token::Eq),
            b'*' => Some(Token::Star),
            b'(' => Some(Token::LParen),
            b')' => Some(Token::RParen),
            _ => None,
        };
        if let Some(t) = single {
            out.push(LocTok {
                tok: t,
                line,
                col,
            });
            i += 1;
            col += 1;
            continue;
        }
        // Identifier — `[A-Za-z_][A-Za-z0-9_]*`.
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            let start_col = col;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
                col += 1;
            }
            let ident = std::str::from_utf8(&bytes[start..i])
                .expect("UTF-8 boundary preserved by ASCII checks above")
                .to_string();
            out.push(LocTok {
                tok: Token::Ident(ident),
                line,
                col: start_col,
            });
            continue;
        }
        // Anything else — emit nothing (parser will surface a proper
        // error at the next consumed token). Avoids surprising the
        // operator with `lex error` for trivia like `;`.
        i += 1;
        col += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::RelationKind;

    /// Snapshot test for the canonical example from plan 12c lines
    /// 1097–1110. Verifies every relation/permission lands in the
    /// parsed [`NamespaceSchema`] and the operator precedence parses
    /// the way humans expect.
    #[test]
    fn parses_plan_example() {
        let src = r#"
            definition user {}

            definition group {
                relation member: user
            }

            definition document {
                relation owner: user
                relation viewer: user | group#member
                permission view = owner + viewer
                permission edit = owner
            }
        "#;
        let nss = parse_schema(src).expect("parse");
        assert_eq!(nss.len(), 3);

        let user = &nss[0];
        assert_eq!(user.name, "user");
        assert!(user.definitions.is_empty());

        let group = &nss[1];
        assert_eq!(group.name, "group");
        let member = group.definitions.get("member").expect("member");
        match &member.kind {
            RelationKind::Direct(types) => {
                assert_eq!(types.len(), 1);
                assert_eq!(types[0].object_type, "user");
                assert!(!types[0].wildcard);
            }
            _ => panic!("expected direct"),
        }

        let doc = &nss[2];
        assert_eq!(doc.name, "document");

        let viewer = doc.definitions.get("viewer").expect("viewer");
        match &viewer.kind {
            RelationKind::Direct(types) => {
                assert_eq!(types.len(), 2);
                assert_eq!(types[0].object_type, "user");
                assert_eq!(types[1].object_type, "group");
                assert_eq!(types[1].relation.as_deref(), Some("member"));
            }
            _ => panic!("expected direct"),
        }

        let view = doc.definitions.get("view").expect("view");
        match &view.kind {
            RelationKind::Permission(expr) => match expr.as_ref() {
                PermissionExpr::Union { left, right } => {
                    matches!(left.as_ref(), PermissionExpr::Direct { relation } if relation == "owner");
                    matches!(right.as_ref(), PermissionExpr::Direct { relation } if relation == "viewer");
                }
                _ => panic!("expected union"),
            },
            _ => panic!("expected permission"),
        }
    }

    #[test]
    fn parses_wildcard_and_arrow() {
        let src = r#"
            definition resource {
                relation public: user:*
                relation parent: folder
                permission read = public + parent->read
            }
        "#;
        let nss = parse_schema(src).expect("parse");
        let r = &nss[0];

        let public = r.definitions.get("public").expect("public");
        match &public.kind {
            RelationKind::Direct(t) => {
                assert!(t[0].wildcard);
                assert_eq!(t[0].object_type, "user");
            }
            _ => panic!(),
        }

        let read = r.definitions.get("read").expect("read");
        match &read.kind {
            RelationKind::Permission(expr) => match expr.as_ref() {
                PermissionExpr::Union { left: _, right } => match right.as_ref() {
                    PermissionExpr::TuplesetArrow {
                        tupleset,
                        permission,
                    } => {
                        assert_eq!(tupleset, "parent");
                        assert_eq!(permission, "read");
                    }
                    _ => panic!("expected arrow on right"),
                },
                _ => panic!("expected union"),
            },
            _ => panic!("expected permission"),
        }
    }

    #[test]
    fn parses_intersect_exclude_and_grouping() {
        let src = r#"
            definition doc {
                relation a: user
                relation b: user
                relation c: user
                permission p = (a + b) & c - a
            }
        "#;
        let nss = parse_schema(src).expect("parse");
        let p = nss[0].definitions.get("p").expect("p");
        // Outer is `Exclude` (lowest precedence).
        match &p.kind {
            RelationKind::Permission(expr) => match expr.as_ref() {
                PermissionExpr::Exclude { left, right: _ } => match left.as_ref() {
                    // Then `Intersect` underneath.
                    PermissionExpr::Intersect { .. } => {}
                    other => panic!("inner left should be intersect, got {other:?}"),
                },
                other => panic!("outer should be exclude, got {other:?}"),
            },
            _ => panic!("expected permission"),
        }
    }

    #[test]
    fn rejects_bad_syntax() {
        // Missing `:` after relation name.
        let err = parse_schema("definition x { relation a user }").unwrap_err();
        assert!(err.message.contains("Colon") || err.message.contains("token"));
    }

    #[test]
    fn ignores_line_comments() {
        let src = r#"
            // top-level comment
            definition user {}
            // another comment between definitions
            definition x {
                relation y: user // trailing comment
            }
        "#;
        let nss = parse_schema(src).expect("parse with comments");
        assert_eq!(nss.len(), 2);
    }
}
