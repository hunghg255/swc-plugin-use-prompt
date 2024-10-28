use swc_core::{
    common::DUMMY_SP,
    ecma::{
        ast::{
            BlockStmt, Expr, ExprOrSpread, ExprStmt, Ident, Lit, NewExpr, Program, Stmt, ThrowStmt,
        },
        visit::{as_folder, FoldWith, VisitMut, VisitMutWith},
    },
    plugin::{plugin_transform, proxies::TransformPluginProgramMetadata},
};
use wasix::{fd_prestat_get, fd_write, wasi::ERRNO_BADF, x::FD_STDOUT, Ciovec, Fd, FD_STDERR};
pub struct TransformVisitor;

fn write(fd: Fd, msg: &str) {
    let iov = Ciovec {
        buf: msg.as_ptr(),
        buf_len: msg.len(),
    };
    unsafe {
        fd_write(fd, &[iov]).expect("failed to write message");
    }
}

fn get_working_directory_fd() -> Option<Fd> {
    unsafe {
        for fd in 3..u32::MAX {
            let res = fd_prestat_get(fd);
            match res {
                Ok(_) => return Some(fd),
                Err(ERRNO_BADF) => {
                    write(FD_STDERR, "no directory open");
                    break;
                }
                Err(_) => {
                    continue;
                }
            }
        }
    };
    return None;
}

enum ParseMaybePromptError {
    NotPrompt,
    PromptEmpty,
}
/// Parse the stmt and extract the prompt to use (if present).
fn parse_maybe_prompt(stmt: &Stmt) -> Result<String, ParseMaybePromptError> {
    let literal = stmt
        .as_expr()
        .ok_or(ParseMaybePromptError::NotPrompt)?
        .expr
        .as_lit()
        .ok_or(ParseMaybePromptError::NotPrompt)?;
    let Lit::Str(str) = literal else {
        Err(ParseMaybePromptError::NotPrompt)?
    };
    let str = str.value.as_str();

    if !str.starts_with("use prompt:") {
        Err(ParseMaybePromptError::NotPrompt)?;
    };
    let prompt = (&str[11..]).trim().to_owned();
    if prompt.is_empty() {
        Err(ParseMaybePromptError::PromptEmpty)?;
    };

    return Ok(prompt);
}

impl VisitMut for TransformVisitor {
    fn visit_mut_fn_decl(&mut self, node: &mut swc_core::ecma::ast::FnDecl) {
        node.visit_mut_children_with(self);

        let Some(body) = &node.function.body else {
            return;
        };
        if body.stmts.is_empty() {
            return;
        };
        let prompt = match parse_maybe_prompt(&body.stmts[0]) {
            Err(ParseMaybePromptError::PromptEmpty) => {
                let expr = ThrowStmt {
                    arg: Box::new(
                        NewExpr {
                            callee: Box::new(Ident::from("Error").into()),
                            args: Some(vec![ExprOrSpread {
                                expr: Box::new(Expr::Lit(Lit::Str("🤖 Incomplete prompt!".into()))),
                                spread: None,
                            }]),
                            ..Default::default()
                        }
                        .into(),
                    ),
                    ..Default::default()
                };
                node.function.body = Some(BlockStmt {
                    stmts: vec![expr.into()],
                    ..Default::default()
                });
                write(
                    FD_STDOUT,
                    &format!("missing prompt {}\n", node.ident.to_string()),
                );
                return;
            }
            Err(_) => {
                return;
            }
            Ok(prompt) => prompt,
        };

        write(
            FD_STDOUT,
            &format!("found prompt {}: {prompt}\n", node.ident.to_string()),
        );

        let expr = ExprStmt {
            span: DUMMY_SP,
            expr: Box::new(prompt.into()),
        };
        let body = BlockStmt {
            stmts: vec![expr.into()],
            ..Default::default()
        };
        node.function.body = Some(body);
    }
}

#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    program.fold_with(&mut as_folder(TransformVisitor))
}
