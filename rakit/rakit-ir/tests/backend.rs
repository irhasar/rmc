use rakit_ir::backend::exec::run as run_machine;
use rakit_ir::backend::isel::RirSelector;
use rakit_ir::backend::mach::{Operand, to_rir};
use rakit_ir::backend::regalloc::LinearScan;
use rakit_ir::backend::{InstructionSelector, RegisterAllocator};
use rakit_ir::ir::{BinOp, IntCc, Type};
use rakit_ir::{Builder, Val, run, verify};

fn select_alloc(func: &rakit_ir::Function) -> rakit_ir::backend::MachFunction {
    let selector = RirSelector;
    let mut mach = selector.select(func).expect("selection");
    let allocator = LinearScan;
    let stats = allocator.allocate(&mut mach).expect("allocation");
    assert!(stats.registers > 0);
    for block in &mach.blocks {
        for inst in &block.insts {
            for operand in inst_operands(inst) {
                assert!(!matches!(operand, Operand::VReg(_)), "unallocated {operand}");
            }
        }
    }
    mach
}

fn inst_operands(inst: &rakit_ir::backend::mach::MachInst) -> Vec<Operand> {
    use rakit_ir::backend::mach::MachInst::*;
    match inst {
        Mov { dst, src } => vec![*dst, *src],
        Bin { dst, lhs, rhs, .. } => vec![*dst, *lhs, *rhs],
        Cmp { lhs, rhs } => vec![*lhs, *rhs],
        SetCC { dst, .. } => vec![*dst],
        Select { dst, cond, then, otherwise } => vec![*dst, *cond, *then, *otherwise],
        Ret { value } => value.iter().copied().collect(),
        StackAddr { dst, .. } | Load { dst, .. } | GlobalValue { dst, .. } | FuncAddr { dst, .. }
        | DynamicStackAddr { dst, .. } => vec![*dst],
        Store { addr, value, .. } => vec![*addr, *value],
        Call { results, args, .. } => results.iter().chain(args).copied().collect(),
        CallIndirect { results, callee, args, .. } => {
            results.iter().chain(std::iter::once(callee)).chain(args).copied().collect()
        }
        Jcc { .. } | Jmp { .. } | Trap(..) => Vec::new(),
    }
}

#[test]
fn selects_and_allocates_arithmetic() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let two = b.iconst(e, Type::I64, 2);
    let scaled = b.bin(e, BinOp::Mul, Type::I64, x, two);
    let result = b.bin(e, BinOp::Add, Type::I64, scaled, y);
    b.ret(e, vec![result]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let expected = run(&f, &[Val::I(5), Val::I(3)]).unwrap();
    let mach = select_alloc(&f);
    assert_eq!(run_machine(&mach, &[5, 3]).unwrap(), Some(13));
    assert_eq!(expected, vec![Val::I(13)]);

    let prealloc = RirSelector.select(&f).unwrap();
    let emitted = to_rir(&prealloc).expect("machine emitter");
    assert!(verify(&emitted).is_ok());
    assert_eq!(run(&emitted, &[Val::I(5), Val::I(3)]).unwrap(), vec![Val::I(13)]);
}

#[test]
fn traits_are_object_safe() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    b.ret(e, vec![x]);
    let f = b.into_func();

    let selector: &dyn InstructionSelector = &RirSelector;
    let mut mach = selector.select(&f).unwrap();
    let allocator: &dyn RegisterAllocator = &LinearScan;
    let stats = allocator.allocate(&mut mach).unwrap();
    assert_eq!(stats.vregs, 1);
    assert_eq!(run_machine(&mach, &[7]).unwrap(), Some(7));
}

#[test]
fn selects_branch_and_runs_both_arms() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let cond = b.icmp(e, IntCc::Slt, Type::I64, x, y);
    let then = b.add_block(vec![]);
    let otherwise = b.add_block(vec![]);
    b.brif(e, cond, then, vec![], otherwise, vec![]);
    b.ret(then, vec![x]);
    b.ret(otherwise, vec![y]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let mach = select_alloc(&f);
    assert_eq!(run_machine(&mach, &[3, 9]).unwrap(), Some(3));
    assert_eq!(run_machine(&mach, &[9, 3]).unwrap(), Some(3));
    assert_eq!(run(&f, &[Val::I(3), Val::I(9)]).unwrap(), vec![Val::I(3)]);
    let prealloc = RirSelector.select(&f).unwrap();
    let emitted = to_rir(&prealloc).expect("machine emitter");
    assert!(verify(&emitted).is_ok(), "{:?}", verify(&emitted));
    assert_eq!(run(&emitted, &[Val::I(3), Val::I(9)]).unwrap(), vec![Val::I(3)]);
}

#[test]
fn spills_under_register_pressure() {
    let params: Vec<Type> = (0..12).map(|_| Type::I64).collect();
    let mut b = Builder::new(vec![Type::I64], params);
    let e = b.func().entry;
    let values = b.func().block(e).params.clone();
    let mut acc = values[0];
    for value in &values[1..] {
        acc = b.bin(e, BinOp::Add, Type::I64, acc, *value);
    }
    b.ret(e, vec![acc]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let selector = RirSelector;
    let mut mach = selector.select(&f).unwrap();
    let stats = LinearScan.allocate(&mut mach).unwrap();
    assert!(stats.spills > 0, "expected spills, got {stats:?}");
    assert!(mach.spill_slots > 0);

    let args: Vec<i64> = (1..=12).collect();
    assert_eq!(run_machine(&mach, &args).unwrap(), Some(78));
}

#[test]
fn selects_memory_and_falls_back_for_i128() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let loaded = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![loaded]);
    let f = b.into_func();
    let selected = RirSelector.select(&f).unwrap();
    assert!(!selected.is_fallback());
    let emitted = rakit_ir::backend::to_rir(&selected).unwrap();
    assert!(verify(&emitted).is_ok(), "{:?}", verify(&emitted));
    assert_eq!(run(&emitted, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);

    let mut b = Builder::new(vec![Type::I128], vec![Type::I128]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    b.ret(e, vec![x]);
    let f = b.into_func();
    let selected = RirSelector.select(&f).unwrap();
    assert!(selected.is_fallback());
}

#[test]
fn i128_arithmetic_and_compare() {
    let mut b = Builder::new(vec![Type::I128], vec![Type::I128, Type::I128]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let sum = b.bin(e, BinOp::Add, Type::I128, x, y);
    b.ret(e, vec![sum]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());
    assert_eq!(run(&f, &[Val::I128(u128::MAX), Val::I128(2)]).unwrap(), vec![Val::I128(1)]);

    let mut b = Builder::new(vec![Type::Bool], vec![Type::I128, Type::I128]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let cond = b.icmp(e, IntCc::Ult, Type::I128, x, y);
    b.ret(e, vec![cond]);
    let f = b.into_func();
    assert_eq!(run(&f, &[Val::I128(5), Val::I128(9)]).unwrap(), vec![Val::B(true)]);
    assert_eq!(run(&f, &[Val::I128(9), Val::I128(5)]).unwrap(), vec![Val::B(false)]);
}

#[test]
fn selects_block_parameters_and_branch_arguments() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let join = b.add_block(vec![Type::I64]);
    b.jump(e, join, vec![x]);
    let p = b.func().block(join).params[0];
    b.ret(join, vec![p]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let mach = select_alloc(&f);
    assert_eq!(run_machine(&mach, &[41]).unwrap(), Some(41));
    let emitted = to_rir(&RirSelector.select(&f).unwrap()).expect("machine emitter");
    assert!(verify(&emitted).is_ok(), "{:?}", verify(&emitted));
    assert_eq!(run(&emitted, &[Val::I(41)]).unwrap(), vec![Val::I(41)]);

    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let then = b.add_block(vec![Type::I64]);
    let otherwise = b.add_block(vec![Type::I64]);
    let zero = b.iconst(e, Type::I64, 0);
    let cond = b.icmp(e, IntCc::Slt, Type::I64, x, zero);
    b.brif(e, cond, then, vec![zero], otherwise, vec![x]);
    let tp = b.func().block(then).params[0];
    let ep = b.func().block(otherwise).params[0];
    b.ret(then, vec![tp]);
    b.ret(otherwise, vec![ep]);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let mach = select_alloc(&f);
    assert_eq!(run_machine(&mach, &[5]).unwrap(), Some(5));
    assert_eq!(run_machine(&mach, &[-5]).unwrap(), Some(0));
    let emitted = to_rir(&RirSelector.select(&f).unwrap()).expect("machine emitter");
    assert!(verify(&emitted).is_ok(), "{:?}", verify(&emitted));
    assert_eq!(run(&emitted, &[Val::I(5)]).unwrap(), vec![Val::I(5)]);
    assert_eq!(run(&emitted, &[Val::I(-5)]).unwrap(), vec![Val::I(0)]);
}

#[test]
fn edge_moves_survive_swapped_loop_parameters() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let t = b.iconst(e, Type::Bool, 1);
    let header = b.add_block(vec![Type::I64, Type::I64, Type::Bool]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    b.jump(e, header, vec![x, y, t]);
    let hp = b.func().block(header).params.clone();
    b.brif(header, hp[2], body, vec![], exit, vec![]);
    let f0 = b.iconst(body, Type::Bool, 0);
    b.jump(body, header, vec![hp[1], hp[0], f0]);
    let ten = b.iconst(exit, Type::I64, 10);
    let a10 = b.bin(exit, BinOp::Mul, Type::I64, hp[0], ten);
    let sum = b.bin(exit, BinOp::Add, Type::I64, a10, hp[1]);
    b.ret(exit, vec![sum]);
    let f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(1), Val::I(2)]).unwrap(), vec![Val::I(21)]);

    let mach = select_alloc(&f);
    assert_eq!(run_machine(&mach, &[1, 2]).unwrap(), Some(21));
}
