//! A reference interpreter for Rakit IR, used by tests to check that the IR and
//! the optimizer preserve semantics. It covers the scalar subset.

use crate::int_sem;
use crate::ir::{BinOp, Block, FloatCc, Function, Inst, Type, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    I(i64),
    I128(u128),
    F32(f32),
    F64(f64),
    B(bool),
    P(u64),
}

impl Val {
    fn as_i(self) -> i64 {
        match self {
            Val::I(v) => v,
            Val::I128(v) => v as i64,
            Val::P(v) => v as i64,
            Val::B(b) => b as i64,
            Val::F32(f) => f as i64,
            Val::F64(f) => f as i64,
        }
    }

    fn as_u64(self) -> u64 {
        match self {
            Val::P(v) => v,
            Val::I128(v) => v as u64,
            _ => self.as_i() as u64,
        }
    }

    fn as_u128(self) -> u128 {
        match self {
            Val::I128(v) => v,
            _ => self.as_u64() as u128,
        }
    }

    fn as_f(self) -> f64 {
        match self {
            Val::I(v) => v as f64,
            Val::I128(v) => v as f64,
            Val::P(v) => v as f64,
            Val::B(b) => b as i64 as f64,
            Val::F32(f) => f as f64,
            Val::F64(f) => f,
        }
    }

    fn as_bool(self) -> bool {
        match self {
            Val::B(b) => b,
            Val::I(v) => v != 0,
            Val::I128(v) => v != 0,
            Val::P(v) => v != 0,
            Val::F32(f) => f != 0.0,
            Val::F64(f) => f != 0.0,
        }
    }
}

fn default_val(ty: Type) -> Val {
    match ty {
        Type::F32 => Val::F32(0.0),
        Type::F64 => Val::F64(0.0),
        Type::Bool => Val::B(false),
        Type::Ptr => Val::P(0),
        Type::I128 => Val::I128(0),
        _ => Val::I(0),
    }
}

fn eval_bin(op: BinOp, ty: Type, a: Val, b: Val) -> Result<Val, String> {
    if ty == Type::I128 {
        return int_sem::eval_bin_i128(op, a.as_u128(), b.as_u128())
            .map(Val::I128)
            .ok_or_else(|| format!("I128 op {op:?} failed (division by zero?)"));
    }
    if ty.is_float() {
        let (x, y) = (a.as_f(), b.as_f());
        let r = match op {
            BinOp::Fadd => x + y,
            BinOp::Fsub => x - y,
            BinOp::Fmul => x * y,
            BinOp::Fdiv => x / y,
            _ => return Err(format!("float op {op:?} on non-float operands")),
        };
        return Ok(match ty {
            Type::F32 => Val::F32(r as f32),
            _ => Val::F64(r),
        });
    }

    if ty == Type::Ptr {
        return int_sem::eval_bin(op, ty, a.as_i(), b.as_i())
            .map(|r| Val::P(r as u64))
            .ok_or_else(|| format!("pointer op {op:?} failed"));
    }

    match int_sem::eval_bin(op, ty, a.as_i(), b.as_i()) {
        Some(r) => Ok(Val::I(r)),
        None => Err(format!("integer op {op:?} failed (division by zero?)")),
    }
}

fn eval_fcmp(cc: FloatCc, a: f64, b: f64) -> bool {
    match cc {
        FloatCc::Eq => a == b,
        FloatCc::Ne => a != b,
        FloatCc::Lt => a < b,
        FloatCc::Le => a <= b,
        FloatCc::Gt => a > b,
        FloatCc::Ge => a >= b,
    }
}

fn get(env: &[Option<Val>], v: Value) -> Result<Val, String> {
    env[v.0 as usize].ok_or_else(|| format!("value {v:?} read before it was defined"))
}

fn size_of(ty: Type) -> usize {
    match ty {
        Type::I8 | Type::Bool => 1,
        Type::I16 => 2,
        Type::I32 | Type::F32 => 4,
        Type::I64 | Type::F64 | Type::Ptr => 8,
        Type::I128 => 16,
    }
}

fn stack_layout(func: &Function) -> (Vec<usize>, Vec<u8>) {
    let mut bases = Vec::with_capacity(func.stack_slots.len());
    let mut total = 0usize;
    for slot in &func.stack_slots {
        let align = slot.align.max(1) as usize;
        total = (total + align - 1) & !(align - 1);
        bases.push(total);
        total += slot.size as usize;
    }
    (bases, vec![0u8; total.max(1)])
}

fn addr_of(base: u64, offset: i32) -> Result<usize, String> {
    let a = base as i64 + offset as i64;
    if a < 0 {
        return Err(format!("negative address {a}"));
    }
    Ok(a as usize)
}

fn read_scalar(mem: &[u8], addr: usize, ty: Type) -> Result<Val, String> {
    let size = size_of(ty);
    let end = addr.checked_add(size).ok_or_else(|| "address overflow".to_string())?;
    if end > mem.len() {
        return Err(format!("load at {addr} (+{size}) is out of bounds"));
    }
    let mut buf = [0u8; 16];
    buf[..size].copy_from_slice(&mem[addr..end]);
    Ok(match ty {
        Type::I8 => Val::I(i64::from(buf[0])),
        Type::I16 => Val::I(i64::from(u16::from_le_bytes(buf[..2].try_into().unwrap()))),
        Type::I32 => Val::I(i64::from(u32::from_le_bytes(buf[..4].try_into().unwrap()))),
        Type::I64 => Val::I(i64::from_le_bytes(buf[..8].try_into().unwrap())),
        Type::I128 => Val::I128(u128::from_le_bytes(buf)),
        Type::F32 => Val::F32(f32::from_le_bytes(buf[..4].try_into().unwrap())),
        Type::F64 => Val::F64(f64::from_le_bytes(buf[..8].try_into().unwrap())),
        Type::Bool => Val::B(buf[0] != 0),
        Type::Ptr => Val::P(u64::from_le_bytes(buf[..8].try_into().unwrap())),
    })
}

fn write_scalar(mem: &mut [u8], addr: usize, ty: Type, val: Val) -> Result<(), String> {
    let size = size_of(ty);
    let end = addr.checked_add(size).ok_or_else(|| "address overflow".to_string())?;
    if end > mem.len() {
        return Err(format!("store at {addr} (+{size}) is out of bounds"));
    }
    let mut buf = [0u8; 16];
    match ty {
        Type::I8 | Type::Bool => buf[0] = val.as_i() as u8,
        Type::I16 => buf[..2].copy_from_slice(&(val.as_i() as i16).to_le_bytes()),
        Type::I32 => buf[..4].copy_from_slice(&(val.as_i() as i32).to_le_bytes()),
        Type::I64 | Type::Ptr => buf[..8].copy_from_slice(&(val.as_u64() as i64).to_le_bytes()),
        Type::I128 => buf = val.as_u128().to_le_bytes(),
        Type::F32 => buf[..4].copy_from_slice(&(val.as_f() as f32).to_le_bytes()),
        Type::F64 => buf[..8].copy_from_slice(&val.as_f().to_le_bytes()),
    }
    mem[addr..end].copy_from_slice(&buf[..size]);
    Ok(())
}

fn eval(func: &crate::ir::Function, env: &[Option<Val>], inst: &Inst, mem: &mut [u8], bases: &[usize]) -> Result<Val, String> {
    match inst {
        Inst::Nop(_, ty) => Ok(default_val(*ty)),
        Inst::Iconst(_, ty, v) => Ok(match ty {
            Type::Bool => Val::B(*v != 0),
            Type::I128 => Val::I128((*v as i128) as u128),
            Type::Ptr => Val::P(*v as u64),
            _ => Val::I(int_sem::wrap(*ty, *v)),
        }),
        Inst::F32const(_, v) => Ok(Val::F32(*v)),
        Inst::F64const(_, v) => Ok(Val::F64(*v)),
        Inst::Copy(_, _, x) => get(env, *x),
        Inst::Call(..) => Err("external call is not supported by the interpreter".to_string()),
        Inst::CallIndirect { .. } => Err("indirect call is not supported by eval".to_string()),
        Inst::TryCall { .. } => Err("try-call is not supported by eval".to_string()),
        Inst::TryCallIndirect { .. } => Err("indirect try-call is not supported by eval".to_string()),
        Inst::GlobalValue(..) => Err("global value is not supported by eval".to_string()),
        Inst::FuncAddr(..) => Err("function address is not supported by eval".to_string()),
        Inst::DynamicStackAddr(..) => Err("dynamic stack address is not supported by eval".to_string()),
        Inst::Trap(code) => Err(format!("trap {}", code)),
        Inst::Unary(_, op, ty, x) => {
            let v = get(env, *x)?;
            match op {
                crate::ir::UnOp::Uextend | crate::ir::UnOp::Bint => {
                    Ok(Val::I(int_sem::wrap(*ty, v.as_u64() as i64)))
                }
                crate::ir::UnOp::Sextend => {
                    let src_ty = func.ty(*x);
                    let signed = if src_ty == Type::I128 {
                        v.as_u128() as i128
                    } else {
                        int_sem::sign_extend(src_ty, v.as_i()) as i128
                    };
                    if *ty == Type::I128 {
                        Ok(Val::I128(signed as u128))
                    } else {
                        Ok(Val::I(int_sem::wrap(*ty, signed as i64)))
                    }
                }
                crate::ir::UnOp::Ireduce => Ok(Val::I(int_sem::wrap(*ty, v.as_i()))),
                crate::ir::UnOp::Bnot => Ok(Val::I(int_sem::wrap(*ty, !v.as_i()))),
                crate::ir::UnOp::Bitcast => Ok(Val::I(int_sem::wrap(*ty, v.as_i()))),
                crate::ir::UnOp::Fpromote => Ok(Val::F64(v.as_f())),
                crate::ir::UnOp::Fdemote => Ok(Val::F32(v.as_f() as f32)),
                crate::ir::UnOp::FcvtFromInt => {
                    let src_ty = func.ty(*x);
                    let signed = if src_ty == Type::I128 {
                        (v.as_u128() as i128) as f64
                    } else {
                        int_sem::sign_extend(src_ty, v.as_i()) as f64
                    };
                    match ty {
                        Type::F32 => Ok(Val::F32(signed as f32)),
                        Type::F64 => Ok(Val::F64(signed)),
                        _ => Err(format!("fcvt_from_sint result type {ty:?}")),
                    }
                }
                crate::ir::UnOp::FcvtFromUint => {
                    let src_ty = func.ty(*x);
                    let unsigned = if src_ty == Type::I128 {
                        v.as_u128() as f64
                    } else {
                        int_sem::wrap(src_ty, v.as_i()) as u64 as f64
                    };
                    match ty {
                        Type::F32 => Ok(Val::F32(unsigned as f32)),
                        Type::F64 => Ok(Val::F64(unsigned)),
                        _ => Err(format!("fcvt_from_uint result type {ty:?}")),
                    }
                }
                crate::ir::UnOp::FcvtToInt => {
                    let f = v.as_f();
                    if !f.is_finite() {
                        return Err(format!("fcvt_to_sint({f}) is NaN or infinite"));
                    }
                    let t = f.trunc();
                    if *ty == Type::I128 {
                        let min = -(2f64.powi(127));
                        let max = 2f64.powi(127);
                        if t < min || t >= max {
                            return Err(format!("fcvt_to_sint({f}) overflows {ty:?}"));
                        }
                        Ok(Val::I128((t as i128) as u128))
                    } else {
                        let bits = int_sem::width(*ty)
                            .ok_or_else(|| format!("fcvt_to_sint result type {ty:?}"))?;
                        let min = -(2f64.powi(bits as i32 - 1));
                        let max = 2f64.powi(bits as i32 - 1);
                        if t < min || t >= max {
                            return Err(format!("fcvt_to_sint({f}) overflows {ty:?}"));
                        }
                        Ok(Val::I(int_sem::wrap(*ty, t as i64)))
                    }
                }
                crate::ir::UnOp::FcvtToUint => {
                    let f = v.as_f();
                    if !f.is_finite() {
                        return Err(format!("fcvt_to_uint({f}) is NaN or infinite"));
                    }
                    let t = f.trunc();
                    if *ty == Type::I128 {
                        if t < 0.0 || t >= 2f64.powi(128) {
                            return Err(format!("fcvt_to_uint({f}) overflows {ty:?}"));
                        }
                        Ok(Val::I128(t as u128))
                    } else {
                        let bits = int_sem::width(*ty)
                            .ok_or_else(|| format!("fcvt_to_uint result type {ty:?}"))?;
                        let max = 2f64.powi(bits as i32);
                        if t < 0.0 || t >= max {
                            return Err(format!("fcvt_to_uint({f}) overflows {ty:?}"));
                        }
                        Ok(Val::I(int_sem::wrap(*ty, t as u64 as i64)))
                    }
                }
            }
        }
        Inst::Bin(_, op, ty, x, y) => eval_bin(*op, *ty, get(env, *x)?, get(env, *y)?),
        Inst::Icmp(_, cc, ty, x, y) => {
            let a = get(env, *x)?;
            let b = get(env, *y)?;
            Ok(Val::B(if *ty == Type::I128 {
                int_sem::eval_icmp_i128(*cc, a.as_u128(), b.as_u128())
            } else {
                int_sem::eval_icmp(*cc, *ty, a.as_i(), b.as_i())
            }))
        }
        Inst::Fcmp(_, cc, _, x, y) => {
            Ok(Val::B(eval_fcmp(*cc, get(env, *x)?.as_f(), get(env, *y)?.as_f())))
        }
        Inst::Select(_, _, c, x, y) => {
            if get(env, *c)?.as_bool() {
                get(env, *x)
            } else {
                get(env, *y)
            }
        }
        Inst::StackAddr(_, slot, offset) => {
            let base = *bases
                .get(slot.0 as usize)
                .ok_or_else(|| format!("stack slot {slot:?} out of range"))?;
            let addr = addr_of(base as u64, *offset)?;
            if addr > mem.len() {
                return Err(format!("stack address {addr} is out of bounds"));
            }
            Ok(Val::P(addr as u64))
        }
        Inst::Load(_, ty, addr, offset, _) => {
            let base = get(env, *addr)?.as_u64();
            read_scalar(mem, addr_of(base, *offset)?, *ty)
        }
        Inst::Store(ty, addr, offset, value, _) => {
            let base = get(env, *addr)?.as_u64();
            write_scalar(mem, addr_of(base, *offset)?, *ty, get(env, *value)?)?;
            Ok(Val::I(0))
        }
        Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) => {
            Err("evaluating a terminator as a value".to_string())
        }
    }
}

/// Execute `func` with `args` and return its return values.
pub fn run(func: &Function, args: &[Val]) -> Result<Vec<Val>, String> {
    run_with(func, args, &|_, _| {
        Err("external call is not supported by the interpreter".to_string())
    })
}

pub type ExternalFn<'a> = dyn Fn(u32, &[Val]) -> Result<Vec<Val>, String> + 'a;
pub type IndirectFn<'a> = dyn Fn(Val, &[Val]) -> Result<Vec<Val>, String> + 'a;

pub fn run_with(
    func: &Function,
    args: &[Val],
    externs: &ExternalFn<'_>,
) -> Result<Vec<Val>, String> {
    run_with_indirect(func, args, externs, &|_, _| {
        Err("indirect call is not supported by the interpreter".to_string())
    })
}

pub fn run_with_indirect(
    func: &Function,
    args: &[Val],
    externs: &ExternalFn<'_>,
    indirect: &IndirectFn<'_>,
) -> Result<Vec<Val>, String> {
    let mut env: Vec<Option<Val>> = vec![None; func.values.len()];
    let (bases, mut mem) = stack_layout(func);

    let entry_params = &func.block(func.entry).params;
    if entry_params.len() != args.len() {
        return Err(format!("expected {} args, got {}", entry_params.len(), args.len()));
    }
    for (p, a) in entry_params.iter().zip(args) {
        env[p.0 as usize] = Some(*a);
    }

    let mut current: Block = func.entry;
    for _ in 0..1_000_000 {
        let blk = func.block(current).clone();
        let mut next: Option<(Block, Vec<Value>)> = None;
        let mut ret: Option<Vec<Value>> = None;

        for inst in &blk.insts {
            match inst {
                Inst::Jump(t, a) => {
                    next = Some((*t, a.clone()));
                    break;
                }
                Inst::Brif(c, t, ta, f, fa) => {
                    let cond = get(&env, *c)?.as_bool();
                    next = Some(if cond { (*t, ta.clone()) } else { (*f, fa.clone()) });
                    break;
                }
                Inst::Return(a) => {
                    ret = Some(a.clone());
                    break;
                }
                Inst::Call(results, target, args) => {
                    let values =
                        args.iter().map(|v| get(&env, *v)).collect::<Result<Vec<_>, _>>()?;
                    let returned = externs(*target, &values)?;
                    if returned.len() != results.len() {
                        return Err(format!(
                            "external call returned {} values for {} results",
                            returned.len(),
                            results.len()
                        ));
                    }
                    for (result, value) in results.iter().zip(returned) {
                        env[result.0 as usize] = Some(value);
                    }
                }
                Inst::CallIndirect { results, callee, args, .. } => {
                    let callee = get(&env, *callee)?;
                    let values =
                        args.iter().map(|v| get(&env, *v)).collect::<Result<Vec<_>, _>>()?;
                    let returned = indirect(callee, &values)?;
                    if returned.len() != results.len() {
                        return Err(format!(
                            "indirect call returned {} values for {} results",
                            returned.len(),
                            results.len()
                        ));
                    }
                    for (result, value) in results.iter().zip(returned) {
                        env[result.0 as usize] = Some(value);
                    }
                }
                _ => {
                    let v = eval(func, &env, inst, &mut mem, &bases)?;
                    if let Some(r) = inst.result_value() {
                        env[r.0 as usize] = Some(v);
                    }
                }
            }
        }

        if let Some(a) = ret {
            return a.iter().map(|v| get(&env, *v)).collect();
        }
        if let Some((t, a)) = next {
            let params = func.block(t).params.clone();
            if params.len() != a.len() {
                return Err(format!(
                    "branch to {t:?} passes {} args for {} params",
                    a.len(),
                    params.len()
                ));
            }
            let mut new_vals = Vec::with_capacity(a.len());
            for v in &a {
                new_vals.push(get(&env, *v)?);
            }
            for (p, v) in params.iter().zip(new_vals) {
                env[p.0 as usize] = Some(v);
            }
            current = t;
        } else {
            return Err(format!("block {current:?} did not end in a terminator"));
        }
    }
    Err("step limit exceeded (possible infinite loop)".to_string())
}
