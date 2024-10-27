use swc_core::{
    common::DUMMY_SP,
    ecma::{
        ast::{BlockStmt, ExprStmt, Program},
        codegen::to_code,
        transforms::testing::test,
        visit::{as_folder, FoldWith, VisitMut, VisitMutWith},
    },
    plugin::{plugin_transform, proxies::TransformPluginProgramMetadata},
};

pub struct TransformVisitor;

impl VisitMut for TransformVisitor {
    fn visit_mut_fn_decl(&mut self, node: &mut swc_core::ecma::ast::FnDecl) {
        node.visit_mut_children_with(self);

        let x = to_code(node);
        let expr = ExprStmt {
            span: DUMMY_SP,
            expr: Box::new("hello world".into()),
        };
        let body = BlockStmt {
            stmts: vec![expr.into()],
            ..Default::default()
        };
        node.function.body = Some(body);
        println!("yay! {x}");
    }
}

#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    program.fold_with(&mut as_folder(TransformVisitor))
}

// An example to test plugin transform.
// Recommended strategy to test plugin's transform is verify
// the Visitor's behavior, instead of trying to run `process_transform` with mocks
// unless explicitly required to do so.
// test_inline!(
//     Default::default(),
//     |_| as_folder(TransformVisitor),
//     boo,
//     // Input codes
//     r#"console.log("transform");"#,
//     // Output codes after transformed with plugin
//     r#"console.log("transform");"#
// );

test!(
    Default::default(),
    |_| as_folder(TransformVisitor),
    wow,
    r#"function MyCoolTest() {
        "use prompt"
    }
    "#
);
