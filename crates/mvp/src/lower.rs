//! OXC AST → tree-IR lowering.
//!
//! `parse_and_lower` owns the parser arena for its whole body, then lowers
//! the AST into our owned IR before the arena is dropped.

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::error::Error;
use crate::ir::tree::*;

pub fn parse_and_lower(source: &str) -> Result<Module, Error> {
    let allocator = Allocator::default();
    let source_type = SourceType::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    if !ret.errors.is_empty() {
        let msg = ret
            .errors
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Parse(msg));
    }
    lower_program(&ret.program)
}

fn lower_program(prog: &js::Program) -> Result<Module, Error> {
    let mut items = Vec::with_capacity(prog.body.len());
    for stmt in &prog.body {
        items.push(lower_top_stmt(stmt)?);
    }
    Ok(Module { items })
}

fn lower_top_stmt(stmt: &js::Statement) -> Result<Item, Error> {
    match stmt {
        js::Statement::FunctionDeclaration(f) => Ok(Item::Function(lower_function(f)?)),
        _ => Ok(Item::Stmt(lower_stmt(stmt)?)),
    }
}

fn lower_function(f: &js::Function) -> Result<IrFunction, Error> {
    let name =
        f.id.as_ref()
            .map(|i| i.name.to_string())
            .ok_or(Error::Unsupported("anonymous function declaration"))?;

    let mut params = Vec::with_capacity(f.params.items.len());
    for p in &f.params.items {
        match &p.pattern.kind {
            js::BindingPatternKind::BindingIdentifier(id) => {
                params.push(id.name.to_string());
            }
            _ => return Err(Error::Unsupported("destructuring or default parameter")),
        }
    }

    let body = match &f.body {
        Some(b) => b
            .statements
            .iter()
            .map(lower_stmt)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    Ok(IrFunction { name, params, body })
}

fn lower_stmt(stmt: &js::Statement) -> Result<Stmt, Error> {
    match stmt {
        js::Statement::VariableDeclaration(decl) => lower_var_decl(decl),
        js::Statement::ExpressionStatement(es) => lower_expr_stmt(&es.expression),
        js::Statement::ReturnStatement(r) => {
            let v = match &r.argument {
                Some(e) => Some(lower_expr(e)?),
                None => None,
            };
            Ok(Stmt::Return(v))
        }
        js::Statement::IfStatement(s) => Ok(Stmt::If {
            cond: lower_expr(&s.test)?,
            then_body: stmts_from(&s.consequent)?,
            else_body: match &s.alternate {
                Some(alt) => Some(stmts_from(alt)?),
                None => None,
            },
        }),
        js::Statement::WhileStatement(s) => Ok(Stmt::While {
            cond: lower_expr(&s.test)?,
            body: stmts_from(&s.body)?,
        }),
        js::Statement::BlockStatement(b) => {
            let stmts = b
                .body
                .iter()
                .map(lower_stmt)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Stmt::Block(stmts))
        }
        js::Statement::FunctionDeclaration(_) => {
            Err(Error::Unsupported("nested function declaration"))
        }
        _ => Err(Error::Unsupported(stmt_kind(stmt))),
    }
}

fn lower_var_decl(decl: &js::VariableDeclaration) -> Result<Stmt, Error> {
    if decl.declarations.len() != 1 {
        return Err(Error::Unsupported("multi-declarator variable declaration"));
    }
    let d = &decl.declarations[0];
    let name = match &d.id.kind {
        js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
        _ => return Err(Error::Unsupported("destructuring binding")),
    };
    let init = match &d.init {
        Some(e) => Some(lower_expr(e)?),
        None => None,
    };
    match decl.kind {
        js::VariableDeclarationKind::Let | js::VariableDeclarationKind::Var => {
            Ok(Stmt::Let { name, init })
        }
        js::VariableDeclarationKind::Const => {
            let init = init.ok_or_else(|| Error::Malformed("const without initializer".into()))?;
            Ok(Stmt::Const { name, init })
        }
        _ => Err(Error::Unsupported("`using` declaration")),
    }
}

fn lower_expr_stmt(expr: &js::Expression) -> Result<Stmt, Error> {
    if let js::Expression::AssignmentExpression(a) = expr {
        if matches!(a.operator, js::AssignmentOperator::Assign) {
            let target = lower_assign_target(&a.left)?;
            let value = lower_expr(&a.right)?;
            return Ok(Stmt::Assign { target, value });
        }
        return Err(Error::Unsupported("compound assignment (+=, -=, etc.)"));
    }
    Ok(Stmt::ExprStmt(lower_expr(expr)?))
}

fn stmts_from(stmt: &js::Statement) -> Result<Vec<Stmt>, Error> {
    if let js::Statement::BlockStatement(b) = stmt {
        b.body.iter().map(lower_stmt).collect()
    } else {
        Ok(vec![lower_stmt(stmt)?])
    }
}

fn lower_assign_target(t: &js::AssignmentTarget) -> Result<Expr, Error> {
    match t {
        js::AssignmentTarget::AssignmentTargetIdentifier(id) => {
            Ok(Expr::Ident(id.name.to_string()))
        }
        js::AssignmentTarget::StaticMemberExpression(m) => Ok(Expr::Member {
            obj: Box::new(lower_expr(&m.object)?),
            prop: m.property.name.to_string(),
        }),
        js::AssignmentTarget::ComputedMemberExpression(m) => Ok(Expr::Index {
            obj: Box::new(lower_expr(&m.object)?),
            index: Box::new(lower_expr(&m.expression)?),
        }),
        _ => Err(Error::Unsupported("complex assignment target")),
    }
}

fn lower_expr(expr: &js::Expression) -> Result<Expr, Error> {
    match expr {
        js::Expression::NumericLiteral(n) => Ok(Expr::Number(n.value)),
        js::Expression::StringLiteral(s) => Ok(Expr::String(s.value.to_string())),
        js::Expression::BooleanLiteral(b) => Ok(Expr::Bool(b.value)),
        js::Expression::NullLiteral(_) => Ok(Expr::Null),
        js::Expression::Identifier(id) => {
            if id.name == "undefined" {
                Ok(Expr::Undefined)
            } else {
                Ok(Expr::Ident(id.name.to_string()))
            }
        }
        js::Expression::ParenthesizedExpression(p) => lower_expr(&p.expression),

        js::Expression::BinaryExpression(b) => Ok(Expr::Binary {
            op: lower_binop(b.operator)?,
            lhs: Box::new(lower_expr(&b.left)?),
            rhs: Box::new(lower_expr(&b.right)?),
        }),
        js::Expression::LogicalExpression(l) => {
            let op = match l.operator {
                js::LogicalOperator::And => BinOp::LogAnd,
                js::LogicalOperator::Or => BinOp::LogOr,
                _ => return Err(Error::Unsupported("nullish coalescing (??)")),
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(lower_expr(&l.left)?),
                rhs: Box::new(lower_expr(&l.right)?),
            })
        }
        js::Expression::UnaryExpression(u) => {
            let op = match u.operator {
                js::UnaryOperator::UnaryNegation => UnOp::Neg,
                js::UnaryOperator::UnaryPlus => UnOp::Pos,
                js::UnaryOperator::LogicalNot => UnOp::Not,
                js::UnaryOperator::BitwiseNot => UnOp::BitNot,
                js::UnaryOperator::Typeof => UnOp::Typeof,
                js::UnaryOperator::Void => UnOp::Void,
                _ => return Err(Error::Unsupported("delete operator")),
            };
            Ok(Expr::Unary {
                op,
                operand: Box::new(lower_expr(&u.argument)?),
            })
        }

        js::Expression::CallExpression(c) => {
            let callee = lower_expr(&c.callee)?;
            let mut args = Vec::with_capacity(c.arguments.len());
            for a in &c.arguments {
                match a {
                    js::Argument::SpreadElement(_) => {
                        return Err(Error::Unsupported("spread argument"));
                    }
                    other => {
                        let e = other
                            .as_expression()
                            .ok_or(Error::Unsupported("non-expression argument"))?;
                        args.push(lower_expr(e)?);
                    }
                }
            }
            Ok(Expr::Call {
                callee: Box::new(callee),
                args,
            })
        }

        js::Expression::StaticMemberExpression(m) => Ok(Expr::Member {
            obj: Box::new(lower_expr(&m.object)?),
            prop: m.property.name.to_string(),
        }),
        js::Expression::ComputedMemberExpression(m) => Ok(Expr::Index {
            obj: Box::new(lower_expr(&m.object)?),
            index: Box::new(lower_expr(&m.expression)?),
        }),

        _ => Err(Error::Unsupported("expression kind")),
    }
}

fn lower_binop(op: js::BinaryOperator) -> Result<BinOp, Error> {
    use js::BinaryOperator as B;
    Ok(match op {
        B::Addition => BinOp::Add,
        B::Subtraction => BinOp::Sub,
        B::Multiplication => BinOp::Mul,
        B::Division => BinOp::Div,
        B::Remainder => BinOp::Mod,
        B::Equality => BinOp::Eq,
        B::StrictEquality => BinOp::StrictEq,
        B::Inequality => BinOp::Ne,
        B::StrictInequality => BinOp::StrictNe,
        B::LessThan => BinOp::Lt,
        B::LessEqualThan => BinOp::Le,
        B::GreaterThan => BinOp::Gt,
        B::GreaterEqualThan => BinOp::Ge,
        B::BitwiseAnd => BinOp::BitAnd,
        B::BitwiseOR => BinOp::BitOr,
        B::BitwiseXOR => BinOp::BitXor,
        B::ShiftLeft => BinOp::Shl,
        B::ShiftRight => BinOp::Shr,
        B::ShiftRightZeroFill => BinOp::UShr,
        _ => return Err(Error::Unsupported("binary operator")),
    })
}

fn stmt_kind(s: &js::Statement) -> &'static str {
    match s {
        js::Statement::ForStatement(_) => "for statement",
        js::Statement::ForInStatement(_) => "for-in statement",
        js::Statement::ForOfStatement(_) => "for-of statement",
        js::Statement::DoWhileStatement(_) => "do-while statement",
        js::Statement::SwitchStatement(_) => "switch statement",
        js::Statement::TryStatement(_) => "try statement",
        js::Statement::ThrowStatement(_) => "throw statement",
        js::Statement::BreakStatement(_) => "break statement",
        js::Statement::ContinueStatement(_) => "continue statement",
        js::Statement::LabeledStatement(_) => "labeled statement",
        js::Statement::ClassDeclaration(_) => "class declaration",
        _ => "unrecognized statement",
    }
}
