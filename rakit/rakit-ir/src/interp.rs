//! A reference interpreter for Rakit IR, used by tests to check that the IR and
//! the optimizer preserve semantics. It covers the scalar subset.

use crate::ir::{BinOp, Block, FloatCc, Function, Inst, IntCc, Type, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    I(i64),
    F32(f32),
    F64(f64),
    B(bool),
    P(u64),
}

impl Val {
    fn as_i(self) -> i64 {
        match self {
            Val::I(v) => v,
            Val::P(v) => v as i64,
            Val::B(b) => b as i64,
            Val::F32(f) => f as i64,
            Val::F64(f) => f as i64,
        }
    }

    fn as_f(self) -> f64 {
        match self {
            Val::I(v) => v as f64,
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
        _ => Val::I(0),
    }
}

fn eval_bin(op: BinOp, ty: Type, a: Val, b: Val) -> Result<Val, String> {
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

    let (x, y) = (a.as_i(), b.as_i());
    let r = match op {
        BinOp::Add => x.wrapping_add(y),
        BinOp::Sub => x.wrapping_sub(y),
        BinOp::Mul => x.wrapping_mul(y),
        BinOp::Sdiv => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            x.wrapping_div(y)
        }
        BinOp::Udiv => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            (x as u64).wrapping_div(y as u64) as i64
        }
        BinOp::Srem => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            x.wrapping_rem(y)
        }
        BinOp::Urem => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            (x as u64).wrapping_rem(y as u64) as i64
        }
        BinOp::And => x & y,
        BinOp::Or => x | y,
        BinOp::Xor => x ^ y,
        BinOp::Shl => x.wrapping_shl((y as u32) & 63),
        BinOp::Shr => x.wrapping_shr((y as u32) & 63),
        BinOp::Ushr => (x as u64).wrapping_shr((y as u32) & 63) as i64,
        _ => return Err(format!("integer op {op:?} on float operands")),
    };
    Ok(Val::I(r))
}

fn eval_icmp(cc: IntCc, a: i64, b: i64) -> bool {
    match cc {
        IntCc::Eq => a == b,
        IntCc::Ne => a != b,
        IntCc::Slt => a < b,
        IntCc::Sle => a <= b,
        IntCc::Sgt => a > b,
        IntCc::Sge => a >= b,
        IntCc::Ult => (a as u64) < (b as u64),
        IntCc::Ule => (a as u64) <= (b as u64),
        IntCc::Ugt => (a as u64) > (b as u64),
        IntCc::Uge => (a as u64) >= (b as u64),
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

fn eval(env: &[Option<Val>], inst: &Inst) -> Result<Val, String> {
    match inst {
        Inst::Nop(_, ty) => Ok(default_val(*ty)),
        Inst::Iconst(_, ty, v) => Ok(if *ty == Type::Bool { Val::B(*v != 0) } else { Val::I(*v) }),
        Inst::F32const(_, v) => Ok(Val::F32(*v)),
        Inst::F64const(_, v) => Ok(Val::F64(*v)),
        Inst::Copy(_, _, x) => get(env, *x),
        Inst::Bin(_, op, ty, x, y) => {
            eval_bin(*op, *ty, get(env, *x)?, get(env, *y)?)
        }
        Inst::Icmp(_, cc, _, x, y) => Ok(Val::B(eval_icmp(*cc, get(env, *x)?.as_i(), get(env, *y)?.as_i()))),
        Inst::Fcmp(_, cc, _, x, y) => Ok(Val::B(eval_fcmp(*cc, get(env, *x)?.as_f(), get(env, *y)?.as_f()))),
        Inst::Select(_, _, c, x, y) => {
            if get(env, *c)?.as_bool() { get(env, *x) } else { get(env, *y) }
        }
        Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) => {
            Err("evaluating a terminator as a value".to_string())
        }
    }
}

/// Execute `func` with `args` and return its return values.
pub fn run(func: &Function, args: &[Val]) -> Result<Vec<Val>, String> {
    let mut env: Vec<Option<Val>> = vec![None; func.values.len()];

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
                _ => {
                    let v = eval(&env, inst)?;
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
                return Err(format!("branch to {t:?} passes {} args for {} params", a.len(), params.len()));
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
