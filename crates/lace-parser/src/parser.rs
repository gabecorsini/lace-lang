use lace_ast::*;
use lace_lexer::{lex, DurationUnit as LexDurationUnit, Span as LexSpan, Token, TokenKind};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    #[error("{message} at {span_start}..{span_end}")]
    Message {
        message: String,
        span_start: usize,
        span_end: usize,
    },
}

/// Convert a byte offset into (1-based line, 1-based column).
pub fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..offset];
    let line = before.chars().filter(|&c| c == '\n').count() + 1;
    let col = before.rfind('\n').map(|p| offset - p - 1).unwrap_or(offset) + 1;
    (line, col)
}

impl ParseError {
    /// Format this error with source context: filename, line/col, offending line, and caret.
    pub fn format_rich(&self, source: &str, filename: &str) -> String {
        match self {
            ParseError::Message { message, span_start, .. } => {
                let (line, col) = offset_to_line_col(source, *span_start);
                let source_line = source.lines().nth(line - 1).unwrap_or("").to_string();
                let caret = " ".repeat(col.saturating_sub(1)) + "^";
                format!(
                    "{filename}:{line}:{col}: error: {message}\n  {source_line}\n  {caret}"
                )
            }
        }
    }
}

pub fn parse_program(source: &str) -> (Option<Program>, Vec<ParseError>) {
    let (tokens, lex_errors) = lex(source);
    let mut errors = Vec::new();
    for e in lex_errors {
        errors.push(ParseError::Message {
            message: e.to_string(),
            span_start: 0,
            span_end: 0,
        });
    }
    if !errors.is_empty() {
        return (None, errors);
    }

    let mut p = Parser {
        tokens,
        pos: 0,
        errors: Vec::new(),
    };
    let program = p.parse_program();
    (program, p.errors)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<ParseError>,
}

impl Parser {
    fn parse_program(&mut self) -> Option<Program> {
        let mut module = None;
        let mut uses = Vec::new();
        let mut imports = Vec::new();
        let mut items = Vec::new();

        if self.at(&TokenKind::Module) {
            module = self.parse_module_decl();
        }

        while self.at(&TokenKind::Use) {
            if let Some(u) = self.parse_use_decl() {
                uses.push(u);
            } else {
                self.synchronize_top_level();
            }
        }

        while self.at(&TokenKind::Import) {
            if let Some(i) = self.parse_import_decl() {
                imports.push(i);
            } else {
                self.synchronize_top_level();
            }
        }

        while !self.at(&TokenKind::Eof) {
            if let Some(item) = self.parse_top_level_item() {
                items.push(item);
            } else {
                self.synchronize_top_level();
                if self.at(&TokenKind::Eof) {
                    break;
                }
            }
        }

        if self.errors.is_empty() {
            Some(Program {
                module,
                uses,
                imports,
                items,
            })
        } else {
            None
        }
    }

    fn parse_module_decl(&mut self) -> Option<ModuleDecl> {
        let start = self.expect(TokenKind::Module)?.span.start;
        let path = self.parse_module_path()?;
        let end = self.prev_span().end;
        Some(ModuleDecl {
            path,
            span: Span { start, end },
        })
    }

    fn parse_use_decl(&mut self) -> Option<UseDecl> {
        let start = self.expect(TokenKind::Use)?.span.start;
        let path = self.parse_module_path()?;
        let imports = if self.match_tok(&TokenKind::Dot) {
            self.expect(TokenKind::LBrace)?;
            let mut names = Vec::new();
            loop {
                names.push(self.expect_ident()?);
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RBrace) {
                        break;
                    }
                    continue;
                }
                break;
            }
            self.expect(TokenKind::RBrace)?;
            Some(names)
        } else {
            None
        };
        let end = self.prev_span().end;
        Some(UseDecl {
            path,
            imports,
            span: Span { start, end },
        })
    }

    fn parse_import_decl(&mut self) -> Option<ImportDecl> {
        let start = self.expect(TokenKind::Import)?.span.start;
        let path = self.parse_module_path()?;
        let end = self.prev_span().end;
        Some(ImportDecl {
            path,
            span: Span { start, end },
        })
    }

    fn parse_top_level_item(&mut self) -> Option<TopLevelItem> {
        let ann = self.parse_annotations();
        match self.peek_kind() {
            TokenKind::Fn => self.parse_fn_decl(ann).map(TopLevelItem::Function),
            TokenKind::Tool => self.parse_tool_decl(ann).map(TopLevelItem::Tool),
            TokenKind::Record => self.parse_record_decl().map(TopLevelItem::Record),
            TokenKind::Enum => self.parse_enum_decl().map(TopLevelItem::Enum),
            TokenKind::Type => self.parse_type_alias().map(TopLevelItem::TypeAlias),
            TokenKind::Const => self.parse_const_decl().map(TopLevelItem::Const),
            TokenKind::Extern => self.parse_extern_decl().map(TopLevelItem::Extern),
            _ => {
                self.error_here("expected top-level item");
                None
            }
        }
    }

    fn parse_fn_decl(&mut self, annotations: Vec<Annotation>) -> Option<FnDecl> {
        let start = self.expect(TokenKind::Fn)?.span.start;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_params();
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let pstart = self.curr_span().start;
                let pname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let pty = self.parse_type_expr()?;
                params.push(Param {
                    name: pname,
                    ty: pty,
                    span: Span {
                        start: pstart,
                        end: self.prev_span().end,
                    },
                });
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;

        let ret_ty = if self.match_tok(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let effects = self.parse_effect_ann()?;
        let body = self.parse_block()?;
        let end = body.span.end;

        Some(FnDecl {
            annotations,
            name,
            generics,
            params,
            ret_ty,
            effects,
            body,
            span: Span { start, end },
        })
    }

    fn parse_tool_decl(&mut self, annotations: Vec<Annotation>) -> Option<ToolDecl> {
        let start = self.expect(TokenKind::Tool)?.span.start;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let pstart = self.curr_span().start;
                let pname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let pty = self.parse_type_expr()?;
                let default = if self.match_tok(&TokenKind::Assign) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                params.push(ToolParam {
                    name: pname,
                    ty: pty,
                    default,
                    span: Span {
                        start: pstart,
                        end: self.prev_span().end,
                    },
                });
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let ret_ty = self.parse_type_expr()?;

        let mut options = Vec::new();
        while matches!(
            self.peek_kind(),
            TokenKind::Retries | TokenKind::Timeout | TokenKind::Mock
        ) {
            let s = self.curr_span().start;
            match self.peek_kind() {
                TokenKind::Retries => {
                    self.bump();
                    self.expect(TokenKind::Colon)?;
                    let v = self.expect_int()?;
                    options.push(ToolOption::Retries(
                        v,
                        Span {
                            start: s,
                            end: self.prev_span().end,
                        },
                    ));
                }
                TokenKind::Timeout => {
                    self.bump();
                    self.expect(TokenKind::Colon)?;
                    let d = self.expect_duration()?;
                    options.push(ToolOption::Timeout(
                        d,
                        Span {
                            start: s,
                            end: self.prev_span().end,
                        },
                    ));
                }
                TokenKind::Mock => {
                    self.bump();
                    self.expect(TokenKind::Colon)?;
                    let name = self.expect_ident()?;
                    options.push(ToolOption::Mock(
                        name,
                        Span {
                            start: s,
                            end: self.prev_span().end,
                        },
                    ));
                }
                _ => break,
            }
        }

        Some(ToolDecl {
            annotations,
            name,
            params,
            ret_ty,
            options,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_record_decl(&mut self) -> Option<RecordDecl> {
        let start = self.expect(TokenKind::Record)?.span.start;
        let name = self.expect_type_ident()?;
        let generics = self.parse_generic_params();
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let fstart = self.curr_span().start;
            let fname = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let fty = self.parse_type_expr()?;
            self.expect(TokenKind::Comma)?;
            fields.push(RecordField {
                name: fname,
                ty: fty,
                span: Span {
                    start: fstart,
                    end: self.prev_span().end,
                },
            });
        }
        self.expect(TokenKind::RBrace)?;
        Some(RecordDecl {
            name,
            generics,
            fields,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_enum_decl(&mut self) -> Option<EnumDecl> {
        let start = self.expect(TokenKind::Enum)?.span.start;
        let name = self.expect_type_ident()?;
        let generics = self.parse_generic_params();
        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let vstart = self.curr_span().start;
            let vname = self.expect_type_ident()?;
            let body = if self.match_tok(&TokenKind::LParen) {
                let mut tys = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        tys.push(self.parse_type_expr()?);
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Some(EnumVariantBody::Tuple(tys))
            } else if self.match_tok(&TokenKind::LBrace) {
                let mut fields = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    let fstart = self.curr_span().start;
                    let fname = self.expect_ident()?;
                    self.expect(TokenKind::Colon)?;
                    let fty = self.parse_type_expr()?;
                    self.expect(TokenKind::Comma)?;
                    fields.push(RecordField {
                        name: fname,
                        ty: fty,
                        span: Span {
                            start: fstart,
                            end: self.prev_span().end,
                        },
                    });
                }
                self.expect(TokenKind::RBrace)?;
                Some(EnumVariantBody::Struct(fields))
            } else {
                None
            };
            self.expect(TokenKind::Comma)?;
            variants.push(EnumVariant {
                name: vname,
                body,
                span: Span {
                    start: vstart,
                    end: self.prev_span().end,
                },
            });
        }
        self.expect(TokenKind::RBrace)?;
        Some(EnumDecl {
            name,
            generics,
            variants,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_type_alias(&mut self) -> Option<TypeAliasDecl> {
        let start = self.expect(TokenKind::Type)?.span.start;
        let name = self.expect_type_ident()?;
        let generics = self.parse_generic_params();
        self.expect(TokenKind::Assign)?;
        let ty = self.parse_type_expr()?;
        Some(TypeAliasDecl {
            name,
            generics,
            ty,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_const_decl(&mut self) -> Option<ConstDecl> {
        let start = self.expect(TokenKind::Const)?.span.start;
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type_expr()?;
        self.expect(TokenKind::Assign)?;
        let expr = self.parse_expr()?;
        Some(ConstDecl {
            name,
            ty,
            expr,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_extern_decl(&mut self) -> Option<ExternDecl> {
        let start = self.expect(TokenKind::Extern)?.span.start;
        self.expect(TokenKind::Fn)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let pstart = self.curr_span().start;
                let pname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let pty = self.parse_type_expr()?;
                params.push(Param {
                    name: pname,
                    ty: pty,
                    span: Span {
                        start: pstart,
                        end: self.prev_span().end,
                    },
                });
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let ret_ty = self.parse_type_expr()?;
        let effects = self.parse_effect_ann()?;
        self.expect(TokenKind::From)?;
        self.expect(TokenKind::Colon)?;
        let source = self.expect_string()?;

        Some(ExternDecl {
            name,
            params,
            ret_ty,
            effects,
            source,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_annotations(&mut self) -> Vec<Annotation> {
        let mut out = Vec::new();
        while self.match_tok(&TokenKind::At) {
            let start = self.prev_span().start;
            let name = self.expect_ident().unwrap_or_else(|| "<error>".into());
            let mut args = Vec::new();
            let mut positional_idx = 0usize;
            if self.match_tok(&TokenKind::LParen) {
                if !self.at(&TokenKind::RParen) {
                    loop {
                        let astart = self.curr_span().start;
                        let (aname, value) = if matches!(self.peek_kind(), TokenKind::Ident(_))
                            && self.peek_n_is(1, &TokenKind::Colon)
                        {
                            let name = self.expect_ident().unwrap_or_else(|| "<error>".into());
                            let _ = self.expect(TokenKind::Colon);
                            let value = self
                                .parse_annotation_value()
                                .unwrap_or(AnnotationValue::String("<error>".into()));
                            (name, value)
                        } else {
                            let value = self
                                .parse_annotation_value()
                                .unwrap_or(AnnotationValue::String("<error>".into()));
                            let name = format!("arg{positional_idx}");
                            positional_idx += 1;
                            (name, value)
                        };

                        args.push(AnnotationArg {
                            name: aname,
                            value,
                            span: Span {
                                start: astart,
                                end: self.prev_span().end,
                            },
                        });
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                let _ = self.expect(TokenKind::RParen);
            }
            out.push(Annotation {
                name,
                args,
                span: Span {
                    start,
                    end: self.prev_span().end,
                },
            });
        }
        out
    }

    fn parse_annotation_value(&mut self) -> Option<AnnotationValue> {
        let kind = self.peek_kind();
        match kind {
            TokenKind::IntLit(v) => {
                self.bump();
                Some(AnnotationValue::Int(v))
            }
            TokenKind::StringLit(s) => {
                self.bump();
                Some(AnnotationValue::String(s))
            }
            TokenKind::BoolLit(v) => {
                self.bump();
                Some(AnnotationValue::Bool(v))
            }
            TokenKind::DurationLit(v, u) => {
                self.bump();
                Some(AnnotationValue::Duration(DurationLit {
                    value: v,
                    unit: map_unit(u),
                }))
            }
            _ => {
                self.error_here("invalid annotation value");
                None
            }
        }
    }

    fn parse_generic_params(&mut self) -> Vec<GenericParam> {
        if !self.match_tok(&TokenKind::Lt) {
            return Vec::new();
        }
        let mut params = Vec::new();
        while !self.at(&TokenKind::Gt) && !self.at(&TokenKind::Eof) {
            let start = self.curr_span().start;
            let (name, kind) = match self.peek_kind() {
                TokenKind::TypeIdent(s) => {
                    self.bump();
                    (s, GenericParamKind::Type)
                }
                TokenKind::Ident(s) => {
                    self.bump();
                    (s, GenericParamKind::Effect)
                }
                _ => {
                    self.error_here("expected generic parameter");
                    break;
                }
            };

            let mut bounds = Vec::new();
            if self.match_tok(&TokenKind::Colon) {
                loop {
                    bounds.push(self.expect_type_ident().unwrap_or_else(|| "<error>".into()));
                    if self.match_tok(&TokenKind::Plus) {
                        continue;
                    }
                    break;
                }
            }

            params.push(GenericParam {
                name,
                kind,
                bounds,
                span: Span {
                    start,
                    end: self.prev_span().end,
                },
            });

            if self.match_tok(&TokenKind::Comma) {
                continue;
            }
            break;
        }
        let _ = self.expect(TokenKind::Gt);
        params
    }

    fn parse_effect_ann(&mut self) -> Option<Vec<EffectExpr>> {
        self.expect(TokenKind::LBracket)?;
        let mut effects = Vec::new();
        if !self.at(&TokenKind::RBracket) {
            loop {
                effects.push(self.parse_effect()?);
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RBracket) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RBracket)?;
        Some(effects)
    }

    fn parse_effect(&mut self) -> Option<EffectExpr> {
        match self.peek_kind() {
            TokenKind::TypeIdent(name) | TokenKind::Ident(name) => {
                self.bump();
                let effect = match name.as_str() {
                    "Pure" => EffectExpr::Builtin(EffectTag::Pure),
                    "IO" => EffectExpr::Builtin(EffectTag::Io),
                    "Mut" => EffectExpr::Builtin(EffectTag::Mut),
                    "ToolCall" => EffectExpr::Builtin(EffectTag::ToolCall),
                    "Time" => EffectExpr::Builtin(EffectTag::Time),
                    "Rand" => EffectExpr::Builtin(EffectTag::Rand),
                    _ => EffectExpr::Variable(name),
                };
                Some(effect)
            }
            _ => {
                self.error_here("expected effect");
                None
            }
        }
    }

    fn parse_type_expr(&mut self) -> Option<TypeExpr> {
        let start = self.curr_span().start;
        match self.peek_kind() {
            TokenKind::Question => {
                self.bump();
                Some(TypeExpr::Dynamic(Span {
                    start,
                    end: self.prev_span().end,
                }))
            }
            TokenKind::Fn => {
                self.bump();
                self.expect(TokenKind::LParen)?;
                let mut params = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        params.push(self.parse_type_expr()?);
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::Arrow)?;
                let ret = self.parse_type_expr()?;
                let effects = if self.at(&TokenKind::LBracket) {
                    self.parse_effect_ann()?
                } else {
                    Vec::new()
                };
                Some(TypeExpr::Function {
                    params,
                    ret: Box::new(ret),
                    effects,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                })
            }
            TokenKind::LParen => {
                self.bump();
                let mut elems = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        elems.push(self.parse_type_expr()?);
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Some(TypeExpr::Tuple {
                    elems,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                })
            }
            TokenKind::TypeIdent(name) => {
                self.bump();
                let end;
                if self.match_tok(&TokenKind::Lt) {
                    let mut args = Vec::new();
                    while !self.at(&TokenKind::Gt) && !self.at(&TokenKind::Eof) {
                        args.push(self.parse_type_expr()?);
                        if self.match_tok(&TokenKind::Comma) {
                            continue;
                        }
                        break;
                    }
                    self.expect(TokenKind::Gt)?;
                    end = self.prev_span().end;
                    Some(TypeExpr::Generic {
                        name,
                        args,
                        span: Span { start, end },
                    })
                } else {
                    end = self.prev_span().end;
                    let prim = match name.as_str() {
                        "Int" => Some(PrimitiveType::Int),
                        "Float" => Some(PrimitiveType::Float),
                        "Bool" => Some(PrimitiveType::Bool),
                        "String" => Some(PrimitiveType::String),
                        "Bytes" => Some(PrimitiveType::Bytes),
                        "Unit" => Some(PrimitiveType::Unit),
                        _ => None,
                    };
                    if let Some(p) = prim {
                        Some(TypeExpr::Primitive(p, Span { start, end }))
                    } else {
                        Some(TypeExpr::Named {
                            name,
                            span: Span { start, end },
                        })
                    }
                }
            }
            _ => {
                self.error_here("expected type expression");
                None
            }
        }
    }

    fn parse_block(&mut self) -> Option<Block> {
        let start = self.expect(TokenKind::LBrace)?.span.start;
        let mut stmts = Vec::new();
        let mut tail_expr = None;

        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Let)
                || (self.at(&TokenKind::Mut) && self.peek_n_is(1, &TokenKind::Let))
                || self.at(&TokenKind::For)
                || self.at(&TokenKind::While)
                || self.at(&TokenKind::Pure)
                || (matches!(self.peek_kind(), TokenKind::Ident(_))
                    && self.peek_n_is(1, &TokenKind::Assign))
            {
                if let Some(stmt) = self.parse_stmt() {
                    stmts.push(stmt);
                } else {
                    self.synchronize_block();
                }
                let _ = self.match_tok(&TokenKind::Semicolon);
                continue;
            }

            let expr = self.parse_expr()?;
            if self.at(&TokenKind::RBrace) {
                tail_expr = Some(Box::new(expr));
                break;
            }
            stmts.push(Stmt::Expr(expr));
            let _ = self.match_tok(&TokenKind::Semicolon);
        }

        self.expect(TokenKind::RBrace)?;
        Some(Block {
            stmts,
            tail_expr,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        match self.peek_kind() {
            TokenKind::Let => {
                let start = self.curr_span().start;
                self.bump();
                let name = self.expect_ident()?;
                let ty = if self.match_tok(&TokenKind::Colon) {
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                self.expect(TokenKind::Assign)?;
                let expr = self.parse_expr()?;
                Some(Stmt::Let(LetStmt {
                    name,
                    ty,
                    expr,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                }))
            }
            TokenKind::Mut => {
                let start = self.curr_span().start;
                self.bump();
                self.expect(TokenKind::Let)?;
                let name = self.expect_ident()?;
                let ty = if self.match_tok(&TokenKind::Colon) {
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                self.expect(TokenKind::Assign)?;
                let expr = self.parse_expr()?;
                Some(Stmt::MutLet(LetStmt {
                    name,
                    ty,
                    expr,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                }))
            }
            TokenKind::For => {
                let start = self.curr_span().start;
                self.bump();
                let name = self.expect_ident()?;
                self.expect(TokenKind::In)?;
                let iter = self.parse_expr()?;
                let body = self.parse_block()?;
                Some(Stmt::For(ForStmt {
                    name,
                    iter,
                    body: body.clone(),
                    span: Span {
                        start,
                        end: body.span.end,
                    },
                }))
            }
            TokenKind::While => {
                let start = self.curr_span().start;
                self.bump();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Some(Stmt::While(WhileStmt {
                    cond,
                    body: body.clone(),
                    span: Span {
                        start,
                        end: body.span.end,
                    },
                }))
            }
            TokenKind::Pure => {
                self.bump();
                let blk = self.parse_block()?;
                Some(Stmt::PureBlock(blk))
            }
            TokenKind::Ident(_) if self.peek_n_is(1, &TokenKind::Assign) => {
                let start = self.curr_span().start;
                let name = self.expect_ident()?;
                self.expect(TokenKind::Assign)?;
                let expr = self.parse_expr()?;
                Some(Stmt::Assign(AssignStmt {
                    name,
                    expr,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                }))
            }
            _ => {
                let expr = self.parse_expr()?;
                Some(Stmt::Expr(expr))
            }
        }
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Option<Expr> {
        let mut lhs = self.parse_prefix_expr()?;

        loop {
            if self.match_tok(&TokenKind::Question) {
                let span = merge_spans(lhs.span(), self.prev_span_ast());
                lhs = Expr::ErrorProp {
                    expr: Box::new(lhs),
                    span,
                };
                continue;
            }

            if self.match_tok(&TokenKind::LParen) {
                if let Expr::Ident(name, s) = lhs {
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.match_tok(&TokenKind::Comma) {
                                if self.at(&TokenKind::RParen) {
                                    break;
                                }
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    lhs = Expr::FnCall(FnCallExpr {
                        name,
                        type_arg: None,
                        args,
                        span: Span {
                            start: s.start,
                            end: self.prev_span().end,
                        },
                    });
                    continue;
                } else {
                    self.error_here("function call target must be identifier");
                    return None;
                }
            }

            if self.match_tok(&TokenKind::ColonColon) {
                let type_arg = self.expect_type_ident()?;
                self.expect(TokenKind::LParen)?;
                let mut args = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                if let Expr::Ident(name, s) = lhs {
                    lhs = Expr::FnCall(FnCallExpr {
                        name,
                        type_arg: Some(type_arg),
                        args,
                        span: Span {
                            start: s.start,
                            end: self.prev_span().end,
                        },
                    });
                    continue;
                }
                self.error_here("qualified call target must be identifier");
                return None;
            }

            if self.match_tok(&TokenKind::Dot) {
                let field_start = self.curr_span().start;
                let name = self.expect_ident()?;
                if self.match_tok(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.match_tok(&TokenKind::Comma) {
                                if self.at(&TokenKind::RParen) {
                                    break;
                                }
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    let span = Span {
                        start: lhs.span().start,
                        end: self.prev_span().end,
                    };
                    lhs = Expr::MethodCall(MethodCallExpr {
                        target: Box::new(lhs),
                        method: name,
                        args,
                        span,
                    });
                } else {
                    let span = Span {
                        start: lhs.span().start,
                        end: self.prev_span().end.max(field_start),
                    };
                    lhs = Expr::FieldAccess {
                        target: Box::new(lhs),
                        field: name,
                        span,
                    };
                }
                continue;
            }

            if self.match_tok(&TokenKind::LBracket) {
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket)?;
                let span = Span {
                    start: lhs.span().start,
                    end: self.prev_span().end,
                };
                lhs = Expr::Index {
                    target: Box::new(lhs),
                    index: Box::new(index),
                    span,
                };
                continue;
            }

            let op = self.peek_kind();
            let (l_bp, r_bp, bop, is_pipe) = match op {
                TokenKind::Star => (17, 18, Some(BinaryOp::Mul), false),
                TokenKind::Slash => (17, 18, Some(BinaryOp::Div), false),
                TokenKind::SlashSlash => (17, 18, Some(BinaryOp::IntDiv), false),
                TokenKind::Percent => (17, 18, Some(BinaryOp::Rem), false),
                TokenKind::Plus => (15, 16, Some(BinaryOp::Add), false),
                TokenKind::Minus => (15, 16, Some(BinaryOp::Sub), false),
                TokenKind::PlusPlus => (15, 16, Some(BinaryOp::Concat), false),
                TokenKind::Lt => (13, 14, Some(BinaryOp::Lt), false),
                TokenKind::Gt => (13, 14, Some(BinaryOp::Gt), false),
                TokenKind::Le => (13, 14, Some(BinaryOp::Le), false),
                TokenKind::Ge => (13, 14, Some(BinaryOp::Ge), false),
                TokenKind::EqEq => (11, 12, Some(BinaryOp::Eq), false),
                TokenKind::NotEq => (11, 12, Some(BinaryOp::Ne), false),
                TokenKind::AndAnd => (9, 10, Some(BinaryOp::And), false),
                TokenKind::OrOr => (7, 8, Some(BinaryOp::Or), false),
                TokenKind::PipeGt => (5, 6, None, true),
                _ => break,
            };

            if l_bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr_bp(r_bp)?;
            let span = Span {
                start: lhs.span().start,
                end: rhs.span().end,
            };
            lhs = if is_pipe {
                Expr::Pipeline {
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                    span,
                }
            } else {
                Expr::Binary {
                    left: Box::new(lhs),
                    op: bop.unwrap(),
                    right: Box::new(rhs),
                    span,
                }
            };
        }

        Some(lhs)
    }

    fn parse_prefix_expr(&mut self) -> Option<Expr> {
        let start = self.curr_span().start;
        match self.peek_kind() {
            TokenKind::Minus => {
                self.bump();
                let expr = self.parse_expr_bp(19)?;
                let span = Span {
                    start,
                    end: expr.span().end,
                };
                Some(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span,
                })
            }
            TokenKind::Bang => {
                self.bump();
                let expr = self.parse_expr_bp(19)?;
                let span = Span {
                    start,
                    end: expr.span().end,
                };
                Some(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                    span,
                })
            }
            TokenKind::If => self.parse_if_expr(),
            TokenKind::Match => self.parse_match_expr(),
            TokenKind::LBrace => self.parse_block().map(Expr::Block),
            TokenKind::Return => {
                self.bump();
                let value = if self.at(&TokenKind::RBrace) {
                    None
                } else {
                    Some(Box::new(self.parse_expr()?))
                };
                Some(Expr::Return {
                    value,
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                })
            }
            TokenKind::Break => {
                self.bump();
                Some(Expr::Break {
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                })
            }
            TokenKind::Continue => {
                self.bump();
                Some(Expr::Continue {
                    span: Span {
                        start,
                        end: self.prev_span().end,
                    },
                })
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::LParen => self.parse_paren_or_tuple(),
            TokenKind::Fn => self.parse_closure_expr(),
            TokenKind::TypeIdent(_) => self.parse_type_ident_expr(),
            TokenKind::IntLit(v) => {
                self.bump();
                Some(Expr::Literal(Literal::Int(v), self.prev_span_ast()))
            }
            TokenKind::FloatLit(v) => {
                self.bump();
                Some(Expr::Literal(Literal::Float(v), self.prev_span_ast()))
            }
            TokenKind::StringLit(v) => {
                self.bump();
                Some(Expr::Literal(Literal::String(v), self.prev_span_ast()))
            }
            TokenKind::BoolLit(v) => {
                self.bump();
                Some(Expr::Literal(Literal::Bool(v), self.prev_span_ast()))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Some(Expr::Ident(name, self.prev_span_ast()))
            }
            _ => {
                self.error_here("expected expression");
                None
            }
        }
    }

    fn parse_closure_expr(&mut self) -> Option<Expr> {
        let start = self.expect(TokenKind::Fn)?.span.start;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                let pstart = self.curr_span().start;
                let pname = self.expect_ident()?;
                let ty = if self.match_tok(&TokenKind::Colon) {
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                params.push(ClosureParam {
                    name: pname,
                    ty,
                    span: Span { start: pstart, end: self.prev_span().end },
                });
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        let ret_ty = if self.match_tok(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        // Optional effects like [Pure]
        let effects = if self.at(&TokenKind::LBracket) {
            self.parse_effect_ann()?
        } else {
            Vec::new()
        };
        let body = self.parse_block()?;
        let end = body.span.end;
        Some(Expr::Closure(ClosureExpr {
            params,
            ret_ty,
            effects,
            body,
            span: Span { start, end },
        }))
    }

    fn parse_if_expr(&mut self) -> Option<Expr> {        let start = self.expect(TokenKind::If)?.span.start;
        let mut branches = Vec::new();
        let cond = self.parse_expr()?;
        let block = self.parse_block()?;
        branches.push((cond, block));

        while self.match_tok(&TokenKind::Else) && self.match_tok(&TokenKind::If) {
            let cond = self.parse_expr()?;
            let blk = self.parse_block()?;
            branches.push((cond, blk));
        }

        let else_block = if self.prev_kind_is(&TokenKind::Else) {
            Some(self.parse_block()?)
        } else {
            None
        };

        let end = if let Some(b) = &else_block {
            b.span.end
        } else {
            branches.last().map(|(_, b)| b.span.end).unwrap_or(start)
        };

        Some(Expr::If(IfExpr {
            branches,
            else_block,
            span: Span { start, end },
        }))
    }

    fn parse_match_expr(&mut self) -> Option<Expr> {
        let start = self.expect(TokenKind::Match)?.span.start;
        let expr = self.parse_expr()?;
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            let astart = self.curr_span().start;
            let pat = self.parse_pattern()?;
            self.expect(TokenKind::FatArrow)?;
            let arm_expr = self.parse_expr()?;
            self.expect(TokenKind::Comma)?;
            arms.push(MatchArm {
                pattern: pat,
                expr: arm_expr,
                span: Span {
                    start: astart,
                    end: self.prev_span().end,
                },
            });
        }
        self.expect(TokenKind::RBrace)?;
        Some(Expr::Match(MatchExpr {
            expr: Box::new(expr),
            arms,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        }))
    }

    fn parse_pattern(&mut self) -> Option<Pattern> {
        let mut pat = self.parse_pattern_atom()?;
        while self.match_tok(&TokenKind::PipeGt) {
            let rhs = self.parse_pattern_atom()?;
            let span = Span {
                start: pat_span(&pat).start,
                end: pat_span(&rhs).end,
            };
            pat = Pattern::Or(Box::new(pat), Box::new(rhs), span);
        }
        Some(pat)
    }

    fn parse_pattern_atom(&mut self) -> Option<Pattern> {
        let start = self.curr_span().start;
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                self.bump();
                if name == "_" {
                    return Some(Pattern::Wildcard(self.prev_span_ast()));
                }
                Some(Pattern::Ident(name, self.prev_span_ast()))
            }
            TokenKind::IntLit(v) => {
                self.bump();
                Some(Pattern::Literal(Literal::Int(v), self.prev_span_ast()))
            }
            TokenKind::FloatLit(v) => {
                self.bump();
                Some(Pattern::Literal(Literal::Float(v), self.prev_span_ast()))
            }
            TokenKind::StringLit(v) => {
                self.bump();
                Some(Pattern::Literal(Literal::String(v), self.prev_span_ast()))
            }
            TokenKind::BoolLit(v) => {
                self.bump();
                Some(Pattern::Literal(Literal::Bool(v), self.prev_span_ast()))
            }
            TokenKind::LParen => {
                self.bump();
                let mut elems = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        elems.push(self.parse_pattern()?);
                        if self.match_tok(&TokenKind::Comma) {
                            if self.at(&TokenKind::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Some(Pattern::Tuple(
                    elems,
                    Span {
                        start,
                        end: self.prev_span().end,
                    },
                ))
            }
            TokenKind::TypeIdent(name) => {
                self.bump();
                if self.match_tok(&TokenKind::LParen) {
                    let mut elems = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        loop {
                            elems.push(self.parse_pattern()?);
                            if self.match_tok(&TokenKind::Comma) {
                                if self.at(&TokenKind::RParen) {
                                    break;
                                }
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    return Some(Pattern::EnumTuple {
                        name,
                        elems,
                        span: Span {
                            start,
                            end: self.prev_span().end,
                        },
                    });
                }
                if self.match_tok(&TokenKind::LBrace) {
                    let mut fields = Vec::new();
                    let mut rest = false;
                    while !self.at(&TokenKind::RBrace) {
                        if self.match_tok(&TokenKind::Dot) {
                            self.expect(TokenKind::Dot)?;
                            rest = true;
                            break;
                        }
                        let fname = self.expect_ident()?;
                        self.expect(TokenKind::Colon)?;
                        let p = self.parse_pattern()?;
                        self.expect(TokenKind::Comma)?;
                        fields.push((fname, p));
                    }
                    self.expect(TokenKind::RBrace)?;
                    return Some(Pattern::Record {
                        name,
                        fields,
                        rest,
                        span: Span {
                            start,
                            end: self.prev_span().end,
                        },
                    });
                }
                Some(Pattern::Ident(name, self.prev_span_ast()))
            }
            _ => {
                self.error_here("expected pattern");
                None
            }
        }
    }

    fn parse_list_literal(&mut self) -> Option<Expr> {
        let start = self.expect(TokenKind::LBracket)?.span.start;
        let mut elems = Vec::new();
        if !self.at(&TokenKind::RBracket) {
            loop {
                elems.push(self.parse_expr()?);
                if self.match_tok(&TokenKind::Comma) {
                    if self.at(&TokenKind::RBracket) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RBracket)?;
        Some(Expr::ListLiteral {
            elems,
            span: Span {
                start,
                end: self.prev_span().end,
            },
        })
    }

    fn parse_paren_or_tuple(&mut self) -> Option<Expr> {
        let start = self.expect(TokenKind::LParen)?.span.start;
        let first = self.parse_expr()?;
        if self.match_tok(&TokenKind::Comma) {
            let mut elems = vec![first];
            if !self.at(&TokenKind::RParen) {
                loop {
                    elems.push(self.parse_expr()?);
                    if self.match_tok(&TokenKind::Comma) {
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
            return Some(Expr::TupleLiteral {
                elems,
                span: Span {
                    start,
                    end: self.prev_span().end,
                },
            });
        }
        self.expect(TokenKind::RParen)?;
        Some(first)
    }

    fn parse_type_ident_expr(&mut self) -> Option<Expr> {
        let start = self.curr_span().start;
        let name = self.expect_type_ident()?;
        if self.match_tok(&TokenKind::LBrace) {
            let mut fields = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                let fstart = self.curr_span().start;
                let fname = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let val = self.parse_expr()?;
                self.expect(TokenKind::Comma)?;
                fields.push((
                    fname,
                    val,
                    Span {
                        start: fstart,
                        end: self.prev_span().end,
                    },
                ));
            }
            self.expect(TokenKind::RBrace)?;
            return Some(Expr::RecordLiteral(RecordLiteralExpr {
                name,
                fields,
                span: Span {
                    start,
                    end: self.prev_span().end,
                },
            }));
        }
        Some(Expr::Ident(name, self.prev_span_ast()))
    }

    fn parse_module_path(&mut self) -> Option<Vec<String>> {
        let mut parts = vec![self.expect_ident()?];
        while self.match_tok(&TokenKind::Dot) {
            parts.push(self.expect_ident()?);
        }
        Some(parts)
    }

    fn expect_ident(&mut self) -> Option<String> {
        match self.peek_kind() {
            TokenKind::Ident(s) => {
                self.bump();
                Some(s)
            }
            _ => {
                self.error_here("expected identifier");
                None
            }
        }
    }

    fn expect_type_ident(&mut self) -> Option<String> {
        match self.peek_kind() {
            TokenKind::TypeIdent(s) => {
                self.bump();
                Some(s)
            }
            _ => {
                self.error_here("expected type identifier");
                None
            }
        }
    }

    fn expect_string(&mut self) -> Option<String> {
        match self.peek_kind() {
            TokenKind::StringLit(s) => {
                self.bump();
                Some(s)
            }
            _ => {
                self.error_here("expected string literal");
                None
            }
        }
    }

    fn expect_int(&mut self) -> Option<i64> {
        match self.peek_kind() {
            TokenKind::IntLit(v) => {
                self.bump();
                Some(v)
            }
            _ => {
                self.error_here("expected integer literal");
                None
            }
        }
    }

    fn expect_duration(&mut self) -> Option<DurationLit> {
        match self.peek_kind() {
            TokenKind::DurationLit(v, u) => {
                self.bump();
                Some(DurationLit {
                    value: v,
                    unit: map_unit(u),
                })
            }
            _ => {
                self.error_here("expected duration literal");
                None
            }
        }
    }

    fn expect(&mut self, expected: TokenKind) -> Option<Token> {
        if self.at(&expected) {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            self.errors.push(ParseError::Message {
                message: format!("expected {:?}, found {:?}", expected, self.peek_kind()),
                span_start: self.curr_span().start,
                span_end: self.curr_span().end,
            });
            None
        }
    }

    fn match_tok(&mut self, expected: &TokenKind) -> bool {
        if self.at(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn at(&self, expected: &TokenKind) -> bool {
        use TokenKind::*;
        matches!(
            (expected, self.peek_kind_ref()),
            (Module, Module)
                | (Use, Use)
                | (Import, Import)
                | (Fn, Fn)
                | (Tool, Tool)
                | (Record, Record)
                | (Enum, Enum)
                | (Type, Type)
                | (Const, Const)
                | (Extern, Extern)
                | (From, From)
                | (Let, Let)
                | (Mut, Mut)
                | (For, For)
                | (In, In)
                | (While, While)
                | (Pure, Pure)
                | (If, If)
                | (Else, Else)
                | (Match, Match)
                | (Return, Return)
                | (Retries, Retries)
                | (Timeout, Timeout)
                | (Mock, Mock)
                | (Plus, Plus)
                | (Minus, Minus)
                | (Star, Star)
                | (Slash, Slash)
                | (Percent, Percent)
                | (EqEq, EqEq)
                | (NotEq, NotEq)
                | (Lt, Lt)
                | (Gt, Gt)
                | (Le, Le)
                | (Ge, Ge)
                | (AndAnd, AndAnd)
                | (OrOr, OrOr)
                | (PlusPlus, PlusPlus)
                | (PipeGt, PipeGt)
                | (Question, Question)
                | (Bang, Bang)
                | (Assign, Assign)
                | (Arrow, Arrow)
                | (FatArrow, FatArrow)
                | (ColonColon, ColonColon)
                | (LParen, LParen)
                | (RParen, RParen)
                | (LBrace, LBrace)
                | (RBrace, RBrace)
                | (LBracket, LBracket)
                | (RBracket, RBracket)
                | (Comma, Comma)
                | (Dot, Dot)
                | (Colon, Colon)
                | (Semicolon, Semicolon)
                | (At, At)
                | (Eof, Eof)
        )
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek_kind_ref().clone()
    }

    fn peek_kind_ref(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_n_is(&self, n: usize, kind: &TokenKind) -> bool {
        let idx = self.pos.saturating_add(n);
        if idx >= self.tokens.len() {
            return false;
        }
        let tmp = Parser {
            tokens: vec![self.tokens[idx].clone()],
            pos: 0,
            errors: Vec::new(),
        };
        tmp.at(kind)
    }

    fn prev_span(&self) -> LexSpan {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn prev_span_ast(&self) -> Span {
        convert_span(self.prev_span())
    }

    fn curr_span(&self) -> LexSpan {
        self.tokens[self.pos].span
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn error_here(&mut self, message: &str) {
        let s = self.curr_span();
        self.errors.push(ParseError::Message {
            message: message.to_string(),
            span_start: s.start,
            span_end: s.end,
        });
    }

    fn synchronize_top_level(&mut self) {
        while !self.at(&TokenKind::Eof) {
            if matches!(
                self.peek_kind(),
                TokenKind::Fn
                    | TokenKind::Tool
                    | TokenKind::Record
                    | TokenKind::Enum
                    | TokenKind::Type
                    | TokenKind::Const
                    | TokenKind::Extern
                    | TokenKind::Use
                    | TokenKind::Import
                    | TokenKind::Module
            ) {
                return;
            }
            self.bump();
        }
    }

    fn synchronize_block(&mut self) {
        while !self.at(&TokenKind::Eof) && !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Semicolon) {
                self.bump();
                return;
            }
            self.bump();
        }
    }

    fn prev_kind_is(&self, kind: &TokenKind) -> bool {
        if self.pos == 0 {
            return false;
        }
        let tmp = Parser {
            tokens: vec![self.tokens[self.pos - 1].clone()],
            pos: 0,
            errors: Vec::new(),
        };
        tmp.at(kind)
    }
}

fn map_unit(unit: LexDurationUnit) -> DurationUnit {
    match unit {
        LexDurationUnit::Ms => DurationUnit::Ms,
        LexDurationUnit::S => DurationUnit::S,
        LexDurationUnit::M => DurationUnit::M,
        LexDurationUnit::H => DurationUnit::H,
    }
}

fn convert_span(s: LexSpan) -> Span {
    Span {
        start: s.start,
        end: s.end,
    }
}

fn merge_spans(a: Span, b: Span) -> Span {
    Span {
        start: a.start,
        end: b.end,
    }
}

fn pat_span(p: &Pattern) -> Span {
    match p {
        Pattern::Wildcard(s)
        | Pattern::Literal(_, s)
        | Pattern::Ident(_, s)
        | Pattern::Tuple(_, s)
        | Pattern::Or(_, _, s)
        | Pattern::EnumTuple { span: s, .. }
        | Pattern::EnumStruct { span: s, .. }
        | Pattern::Record { span: s, .. } => *s,
    }
}
