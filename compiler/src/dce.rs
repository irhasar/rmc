//! Dead-function elimination.
//!
//! Only the functions reachable from `main` can run, so the rest need not be
//! emitted. This is not just an artifact-size saving: the runtime is selected
//! from the emitted assembly (see `runtime::for_program`), so a dead function
//! that calls `print_str` or `alloc` would otherwise drag that runtime piece
//! into the binary as well.
//!
//! The call graph is over-approximate in the safe direction: an edge is added
//! for every function named anywhere in a body, including a call in a branch
//! that never executes. Over-approximating keeps a function that might be
//! needed; under-approximating would drop one that is, so every expression and
//! statement form is walked and a `Call` is the only edge.

use crate::ast::{Block, Expr, FnDef, Program, Stmt};
use std::collections::{HashMap, HashSet};

/// Drop every function not reachable from `main`. When there is no `main` the
/// program is left alone: it cannot link anyway (`_start` calls `main`), and
/// keeping the functions preserves the existing "undefined reference to main"
/// error rather than turning it into an empty translation unit.
pub fn run(prog: &mut Program) {
    if !prog.fns.iter().any(|f| f.name == "main") {
        return;
    }

    // Calls made by each function, as a set of callee names.
    let callees: HashMap<String, HashSet<String>> = prog
        .fns
        .iter()
        .map(|f| (f.name.clone(), calls_in_fn(f)))
        .collect();

    // Mark reachable from `main` to a fixpoint.
    let mut reachable: HashSet<String> = HashSet::new();
    let mut work: Vec<String> = vec!["main".to_string()];
    while let Some(name) = work.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if let Some(cs) = callees.get(&name) {
            work.extend(cs.iter().cloned());
        }
    }

    prog.fns.retain(|f| reachable.contains(&f.name));
}

fn calls_in_fn(f: &FnDef) -> HashSet<String> {
    let mut out = HashSet::new();
    collect_block(&f.body, &mut out);
    out
}

fn collect_block(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        collect_stmt(s, out);
    }
    if let Some(t) = &b.tail {
        collect_expr(t, out);
    }
}

fn collect_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { init, .. } => {
            if let Some(e) = init {
                collect_expr(e, out);
            }
        }
        Stmt::Assign { target, value } => {
            collect_expr(target, out);
            collect_expr(value, out);
        }
        Stmt::Expr(e) => collect_expr(e, out),
        Stmt::While(c, b, _) => {
            collect_expr(c, out);
            collect_block(b, out);
        }
        Stmt::For { start, end, body, .. } => {
            collect_expr(start, out);
            collect_expr(end, out);
            collect_block(body, out);
        }
        Stmt::Return(v) => {
            if let Some(e) = v {
                collect_expr(e, out);
            }
        }
        Stmt::Break(v, _) => {
            if let Some(e) = v {
                collect_expr(e, out);
            }
        }
        Stmt::Continue(_) => {}
    }
}

fn collect_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        Expr::Call { name, args, .. } => {
            out.insert(name.clone());
            for a in args {
                collect_expr(a, out);
            }
        }
        Expr::Bin(_, a, b) => {
            collect_expr(a, out);
            collect_expr(b, out);
        }
        Expr::Un(_, a) | Expr::Cast(a, _) | Expr::Deref { expr: a, .. } => collect_expr(a, out),
        Expr::If(c, t, els) => {
            collect_expr(c, out);
            collect_block(t, out);
            if let Some(e) = els {
                collect_block(e, out);
            }
        }
        Expr::MatchBoundary(b, _) | Expr::Block(b) | Expr::Loop(b, _, _, _) => collect_block(b, out),
        Expr::Index(a, b) => {
            collect_expr(a, out);
            collect_expr(b, out);
        }
        Expr::Field { base, .. } => collect_expr(base, out),
        Expr::StructLit { fields, .. } => {
            for fi in fields {
                collect_expr(&fi.value, out);
            }
        }
        Expr::ArrayLit(v, _) => collect_expr(v, out),
        Expr::ArrayList(vs) => {
            for v in vs {
                collect_expr(v, out);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            collect_expr(recv, out);
            for a in args {
                collect_expr(a, out);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args {
                collect_expr(a, out);
            }
        }
        Expr::VariantTest { scrutinee, .. } | Expr::BoolTest { scrutinee, .. } => {
            collect_expr(scrutinee, out)
        }
        Expr::EnumPayload { recv, .. } => collect_expr(recv, out),
        Expr::AddrOf { expr, .. } => collect_expr(expr, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser, sema};

    fn analyzed(src: &str) -> Program {
        let toks = lexer::lex(src).expect("lex");
        let mut prog = parser::Parser::new(toks).parse_program().expect("parse");
        sema::analyze(&mut prog).expect("sema");
        prog
    }

    /// The function names that survive, sorted for a stable comparison.
    fn kept(src: &str) -> Vec<String> {
        let mut p = analyzed(src);
        run(&mut p);
        let mut names: Vec<String> = p.fns.iter().map(|f| f.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn drops_a_function_main_never_calls() {
        assert_eq!(
            kept("fn dead() -> i64 { return 1; } fn main() -> i64 { return 0; }"),
            vec!["main"]
        );
    }

    #[test]
    fn keeps_a_function_main_calls_transitively() {
        let src = "fn a() -> i64 { return b(); } fn b() -> i64 { return 2; } \
                   fn main() -> i64 { return a(); }";
        assert_eq!(kept(src), vec!["a", "b", "main"]);
    }

    #[test]
    fn keeps_a_callee_named_in_a_branch_that_cannot_run() {
        // Reachability over-approximates on purpose: a call in a dead branch
        // still keeps its callee, so the pass can never drop a live function.
        let src = "fn c() -> i64 { return 3; } \
                   fn main() -> i64 { if false { return c(); } return 0; }";
        assert_eq!(kept(src), vec!["c", "main"]);
    }

    #[test]
    fn keeps_a_callee_named_in_a_loop_body() {
        let src = "fn c() -> i64 { return 3; } \
                   fn main() -> i64 { let mut s: i64 = 0; while s < 3 { s = s + c(); } return s; }";
        assert_eq!(kept(src), vec!["c", "main"]);
    }

    #[test]
    fn drops_an_unreached_method_and_keeps_a_reached_one() {
        let src = "struct P { x: i64 } \
                   impl P { fn get(&self) -> i64 { return self.x; } \
                            fn unused(&self) -> i64 { return 9; } } \
                   fn main() -> i64 { let v: P = P { x: 1 }; return v.get(); }";
        let names = kept(src);
        assert_eq!(names, vec!["P$get", "main"]);
    }

    #[test]
    fn a_program_without_main_is_left_alone() {
        let mut p = analyzed("fn f() -> i64 { return 1; } fn g() -> i64 { return 2; }");
        run(&mut p);
        assert_eq!(p.fns.len(), 2, "no `main` means nothing is pruned");
    }

    #[test]
    fn a_callee_reached_through_a_deeply_nested_expression_is_kept() {
        let src = "fn c() -> i64 { return 1; } \
                   fn main() -> i64 { let a: [i64; 2] = [c(), c()]; return a[0] + a[1]; }";
        assert_eq!(kept(src), vec!["c", "main"]);
    }
}
