use rakit_ir::ir::Type;
use rakit_ir::{Builder, verify};

#[test]
fn reachable_use_of_unreachable_definition_is_rejected() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let entry = b.func().entry;
    let dead = b.add_block(vec![]);
    let v = b.iconst(dead, Type::I64, 7);
    b.ret(dead, vec![v]);
    b.ret(entry, vec![v]);
    assert!(verify(&b.into_func()).is_err());
}
