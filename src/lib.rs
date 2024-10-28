use std::{
    collections::HashMap,
    fs::{self},
    sync::Arc,
};

use serde::Deserialize;
use swc_core::{
    atoms::Atom,
    common::{BytePos, FileName, SourceFile, Span, Spanned, DUMMY_SP},
    ecma::{
        ast::{
            BlockStmt, EsVersion, Expr, ExprOrSpread, ExprStmt, Function, Ident, ImportDecl,
            ImportDefaultSpecifier, ImportPhase, ImportSpecifier, Lit, Module, ModuleDecl,
            ModuleExportName, ModuleItem, NewExpr, Program, Stmt, ThrowStmt,
        },
        parser::{parse_file_as_module, PResult, Syntax::Typescript, TsSyntax},
        visit::{as_folder, FoldWith, VisitMut, VisitMutWith},
    },
    plugin::{plugin_transform, proxies::TransformPluginProgramMetadata},
};

// cwd gets mapped to /cwd by the swc plugin runner.
const PROMPTS_FILE: &str = "/cwd/node_modules/.swc-plugin-use-prompt/prompts";

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

fn make_module_from_source(source: &str) -> PResult<Module> {
    let source_file = SourceFile::new(
        Arc::from(FileName::Anon),
        false,
        Arc::from(FileName::Anon),
        source.to_owned(),
        BytePos(0),
    );
    let mut errors = vec![];

    parse_file_as_module(
        &source_file,
        Typescript(TsSyntax {
            tsx: true,
            ..Default::default()
        }),
        EsVersion::EsNext,
        None,
        &mut errors,
    )
}

type IdentMap = HashMap<Atom, Atom>;

/// Replace Idents with other Idents as specified by IdentMap
struct RenameIdentVisitor {
    pub ident_map: IdentMap,
}

impl RenameIdentVisitor {
    pub fn new(ident_map: IdentMap) -> Self {
        Self { ident_map }
    }
}
impl VisitMut for RenameIdentVisitor {
    fn visit_mut_ident(&mut self, node: &mut Ident) {
        if let Some(replacement) = self.ident_map.get(&node.sym) {
            node.sym = replacement.clone();
        }
    }
}

/// Parse the given source block into a BlockStmt AST. Expected to be a function
/// body.
fn make_block_stmt_from_source(source: &str, ident_map: IdentMap) -> PResult<BlockStmt> {
    let mut ast = make_module_from_source(&format!("function wrapped() {{ {source} }}"))?;
    // This looks scary but it's fine because it's the exact shape we expect.
    // Invalid inputs would have been caught by the `parse_file_as_module` call.
    let decl = ast.body[0]
        .as_mut_stmt()
        .unwrap()
        .as_mut_decl()
        .unwrap()
        .as_mut_fn_decl()
        .unwrap();

    let mut vis = RenameIdentVisitor::new(ident_map);
    vis.visit_mut_fn_decl(decl);
    Ok(decl.function.body.clone().unwrap())
}

/// Rename imports to prefix, and capture renamed things in an IdentMap
struct RenameImportsVisitor {
    prefix: String,
    pub ident_map: IdentMap,
}
impl RenameImportsVisitor {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            ident_map: HashMap::new(),
        }
    }
}
impl VisitMut for RenameImportsVisitor {
    fn visit_mut_import_decl(&mut self, node: &mut swc_core::ecma::ast::ImportDecl) {
        node.specifiers.iter_mut().for_each(|spec| {
            match spec {
                ImportSpecifier::Named(import_named_specifier) => {
                    let key = import_named_specifier.local.sym.clone();
                    let pfxed: Atom = format!("{}{key}", self.prefix).into();
                    self.ident_map.insert(key.clone(), pfxed.clone());
                    import_named_specifier.local.sym = pfxed;
                    import_named_specifier.imported = Some(ModuleExportName::Ident(key.into()));
                }
                ImportSpecifier::Default(import_default_specifier) => {
                    let key = import_default_specifier.local.sym.clone();
                    let pfxed: Atom = format!("{}{key}", self.prefix).into();
                    self.ident_map.insert(key.clone(), pfxed.clone());
                    import_default_specifier.local.sym = pfxed;
                }
                ImportSpecifier::Namespace(import_star_as_specifier) => {
                    let key = import_star_as_specifier.local.sym.clone();
                    let pfxed: Atom = format!("{}{key}", self.prefix).into();
                    self.ident_map.insert(key.clone(), pfxed.clone());
                    import_star_as_specifier.local.sym = pfxed;
                }
            };
        });
    }
}

fn make_imports_from_source(source: &str, prefix: &str) -> PResult<(Vec<ModuleItem>, IdentMap)> {
    let mut ast = make_module_from_source(source)?;
    let mut vis = RenameImportsVisitor::new(prefix);
    vis.visit_mut_module(&mut ast);
    Ok((ast.body, vis.ident_map.clone()))
}

#[derive(Deserialize, Debug)]
struct Substitution {
    code: String,
    imports: Option<String>,
}

type SubstitutionMap = HashMap<String, HashMap<String, HashMap<String, Substitution>>>;

pub struct SubstitutionVisitor {
    substitutions: SubstitutionMap,
    imports: Vec<ModuleItem>,
    visited: u32,
}

impl SubstitutionVisitor {
    pub fn new(cache_file: &str) -> Self {
        let contents = String::from_utf8(fs::read(cache_file).unwrap_or(b"{}".to_vec()))
            .expect("malformed substitutions json");
        let substitutions: SubstitutionMap =
            serde_json::from_str(&contents).expect("malformed substitutions json");

        Self {
            substitutions,
            imports: vec![],
            visited: 0,
        }
    }

    /// Substitute the function body with the codegen'd one, matching using
    /// the span and prompt. (Not perfect, but good enough.)
    fn transform_fn_body(self: &mut Self, func: &mut Function, span: Span) {
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

        let visit_index = self.visited;
        self.visited += 1;

        let subst = match self.substitutions.get(&span.lo.0.to_string()) {
            Some(m) => match m.get(&span.hi.0.to_string()) {
                Some(m) => m.get(&prompt),
                None => None,
            },
            None => None,
        };

        let Some(subst) = subst else {
            println!("⌛ Waiting for component generation...");
            return;
        };

        let mut ident_map: IdentMap = HashMap::new();
        if let Some(imports) = &subst.imports {
            let prefix = format!("__swcPluginUsePromptImport__{visit_index}_");
            match make_imports_from_source(imports, &prefix) {
                Ok((new_imports, new_ident_map)) => {
                    self.imports.extend(new_imports);
                    ident_map = new_ident_map;
                }
                Err(e) => {
                    func.body = make_prompt_error_body(&format!("uh oh: {:?}", e));
                    return;
                }
            }
        };

        match make_block_stmt_from_source(&subst.code, ident_map) {
            Ok(body) => func.body = Some(body),
            Err(e) => {
                func.body =
                    make_prompt_error_body("🤖 Guess ChatGPT isn't great at writing code...");
                println!("Error: {:?}\n", e);
            }
        };
    }
}

impl VisitMut for SubstitutionVisitor {
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

    fn visit_mut_module(&mut self, node: &mut Module) {
        node.visit_mut_children_with(self);

        if self.visited > 0 {
            // ensure "use client"
            let has_client_directive = node
                .body
                .iter()
                .find(|item| match item {
                    ModuleItem::Stmt(stmt) => match stmt {
                        Stmt::Expr(stmt) => match stmt.expr.as_lit() {
                            Some(Lit::Str(str)) => str.value.as_str().eq("use client"),
                            _ => false,
                        },
                        _ => false,
                    },
                    _ => false,
                })
                .is_some();

            if !has_client_directive {
                node.body.insert(
                    0,
                    ModuleItem::Stmt(Stmt::Expr(ExprStmt {
                        span: DUMMY_SP,
                        expr: Box::new(Expr::Lit(Lit::Str("use client".into()))),
                    })),
                );
            }
        }

        node.body.extend(self.imports.clone());
    }
}

struct FixImportsVisitor {
    has_react: bool,
}
impl FixImportsVisitor {
    pub fn new() -> Self {
        Self { has_react: false }
    }
}

impl VisitMut for FixImportsVisitor {
    fn visit_mut_import_decl(&mut self, node: &mut swc_core::ecma::ast::ImportDecl) {
        node.specifiers.iter_mut().for_each(|spec| {
            let sym = match spec {
                ImportSpecifier::Named(import_named_specifier) => &import_named_specifier.local.sym,
                ImportSpecifier::Default(import_default_specifier) => {
                    &import_default_specifier.local.sym
                }
                ImportSpecifier::Namespace(import_star_as_specifier) => {
                    &import_star_as_specifier.local.sym
                }
            };
            if sym.to_string().eq("React") {
                self.has_react = true;
            }
        });
    }

    fn visit_mut_module(&mut self, node: &mut Module) {
        node.visit_mut_children_with(self);

        if !self.has_react {
            node.body
                .push(ModuleItem::ModuleDecl(ModuleDecl::Import(ImportDecl {
                    span: DUMMY_SP,
                    specifiers: vec![ImportSpecifier::Default(ImportDefaultSpecifier {
                        span: DUMMY_SP,
                        local: Ident::from(Atom::from("React")),
                    })],
                    type_only: false,
                    with: None,
                    phase: ImportPhase::Evaluation,
                    src: Box::new("react".into()),
                })));
        }
    }
}

#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    let program = program.fold_with(&mut as_folder(SubstitutionVisitor::new(PROMPTS_FILE)));
    program.fold_with(&mut as_folder(FixImportsVisitor::new()))
}
