use std::{
    collections::HashMap,
    fs::{self},
    sync::Arc,
};

use serde::Deserialize;
use swc_core::{
    common::{BytePos, FileName, SourceFile, Span, Spanned},
    ecma::{
        ast::{
            BlockStmt, EsVersion, Expr, ExprOrSpread, Function, Ident, Lit, NewExpr, Program, Stmt,
            ThrowStmt,
        },
        parser::{parse_file_as_module, PResult, Syntax::Typescript, TsSyntax},
        visit::{as_folder, FoldWith, VisitMut, VisitMutWith},
    },
    plugin::{plugin_transform, proxies::TransformPluginProgramMetadata},
};
use wasix::{fd_write, x::FD_STDOUT, Ciovec, Fd};

fn write(fd: Fd, msg: &str) {
    let iov = Ciovec {
        buf: msg.as_ptr(),
        buf_len: msg.len(),
    };
    unsafe {
        fd_write(fd, &[iov]).expect("failed to write message");
    }
}

/// Generate an error message to be thrown at runtime.
/// TODO: Maybe there's a nice way to throw compile-time errors from SWC Plugins?
fn make_prompt_error_body(msg: &str) -> Option<BlockStmt> {
    let expr = ThrowStmt {
        arg: Box::new(
            NewExpr {
                callee: Box::new(Ident::from("Error").into()),
                args: Some(vec![ExprOrSpread {
                    expr: Box::new(Expr::Lit(Lit::Str(msg.into()))),
                    spread: None,
                }]),
                ..Default::default()
            }
            .into(),
        ),
        ..Default::default()
    };
    Some(BlockStmt {
        stmts: vec![expr.into()],
        ..Default::default()
    })
}

/// Parse the given source block into a BlockStmt AST. Expected to be a function
/// body.
fn make_block_stmt_from_source(source: &str) -> PResult<BlockStmt> {
    let source_file = SourceFile::new(
        Arc::from(FileName::Anon),
        false,
        Arc::from(FileName::Anon),
        format!("function wrapped() {{ {source} }}"),
        BytePos(0),
    );
    let mut errors = vec![];
    let ast = parse_file_as_module(
        &source_file,
        Typescript(TsSyntax {
            tsx: true,
            ..Default::default()
        }),
        EsVersion::EsNext,
        None,
        &mut errors,
    )?;

    // This looks scary but it's fine because it's the exact shape we expect.
    // Invalid inputs would have been caught by the `parse_file_as_module` call.
    let decl = ast.body[0]
        .as_stmt()
        .unwrap()
        .as_decl()
        .unwrap()
        .as_fn_decl()
        .unwrap();
    Ok(decl.function.body.clone().unwrap())
}

#[derive(Deserialize, Debug)]
struct Substitution {
    code: String,
    imports: Option<String>,
}

type SubstitutionMap = HashMap<String, HashMap<String, HashMap<String, Substitution>>>;

pub struct TransformVisitor {
    substitutions: SubstitutionMap,
}

impl TransformVisitor {
    pub fn new(cache_file: &str) -> Self {
        let contents = String::from_utf8(fs::read(cache_file).unwrap_or(b"{}".to_vec()))
            .expect("malformed substitutions json");
        let substitutions: SubstitutionMap =
            serde_json::from_str(&contents).expect("malformed substitutions json");

        Self { substitutions }
    }

    /// Substitute the function body with the codegen'd one, matching using
    /// the span and prompt. (Not perfect, but good enough.)
    fn transform_fn_body(self: &Self, func: &mut Function, span: Span) {
        let Some(body) = &func.body else {
            return;
        };
        if body.stmts.is_empty() {
            return;
        };
        let prologue: Vec<_> = body
            .stmts
            .iter()
            .map_while(|stmt| match stmt {
                Stmt::Expr(expr) => match expr.expr.as_lit() {
                    Some(Lit::Str(str)) => Some(str.value.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let prompt = prologue
            .iter()
            .filter_map(|s| {
                if !s.starts_with("use prompt:") {
                    return None;
                };
                let prompt = (&s[11..]).trim().to_owned();
                if prompt.is_empty() {
                    return Some(Err(1));
                }
                return Some(Ok(prompt));
            })
            .next();

        let Some(prompt) = prompt else {
            return;
        };
        let Ok(prompt) = prompt else {
            func.body = make_prompt_error_body("🤖 Incomplete prompt!");
            return;
        };

        write(
            FD_STDOUT,
            &format!("{} {} {}\n", span.lo.0, span.hi.0, prompt),
        );
        let subst = match self.substitutions.get(&span.lo.0.to_string()) {
            Some(m) => match m.get(&span.hi.0.to_string()) {
                Some(m) => m.get(&prompt),
                None => None,
            },
            None => None,
        };

        let Some(subst) = subst else {
            func.body = make_prompt_error_body(
                "🤖 Missing substitution data. Whoops that's probably my fault.",
            );
            return;
        };

        if let Some(imports) = &subst.imports {
            func.body = make_prompt_error_body(&format!(
                "🤖 It would appear you need to add some imports.\n{imports}"
            ));
            return;
        };

        match make_block_stmt_from_source(&subst.code) {
            Ok(body) => func.body = Some(body),
            Err(e) => {
                func.body =
                    make_prompt_error_body("🤖 Guess ChatGPT isn't great at writing code...");
                write(FD_STDOUT, &format!("Error: {:?}\n", e));
            }
        };
    }
}

impl VisitMut for TransformVisitor {
    fn visit_mut_fn_decl(&mut self, node: &mut swc_core::ecma::ast::FnDecl) {
        node.visit_mut_children_with(self);

        let span = node.span();
        self.transform_fn_body(&mut node.function, span);
    }

    fn visit_mut_fn_expr(&mut self, node: &mut swc_core::ecma::ast::FnExpr) {
        node.visit_mut_children_with(self);

        let span = node.span();
        self.transform_fn_body(&mut node.function, span);
    }
}

#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    program.fold_with(&mut as_folder(TransformVisitor::new(
        // Yeah it's hard-coded. Oh well.
        "/cwd/node_modules/.swc-plugin-use-prompt-cache",
    )))
}
