// A `*const u8`/`*mut u8` names a single byte, so a read or a write touches
// one byte. Treating a byte pointer as a word pointer reads the eight bytes at
// that address (the reported `0x0707070707070707` for a byte of `7`), and a
// word store clobbers the seven bytes after the target.
//
// `u8` zero-extends on load and `i8` sign-extends, so a stored `-5` reads back
// as `-5`, not `251`. A byte pointer cast to a signed one reads the same byte
// signed, and a `&mut u8` borrow writes a single byte.
fn main() {
    // The reported shape: a byte read must be one byte, not a word.
    let v: Vec<u8> = vec![7u8; 8];
    let p = v.as_ptr();
    let q = p as *const u8;
    println!("{}", unsafe { *q });

    // A distinct value per byte: a word-stride read would alias them.
    let mut w: Vec<u8> = vec![0u8; 8];
    let bp = w.as_mut_ptr();
    let mut i: i64 = 0;
    while i < 8 {
        let e = unsafe { bp.add(i as usize) } as *mut u8;
        unsafe { *e = (i * 10 + 1) as u8; }
        i += 1;
    }
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < 8 {
        let e = unsafe { bp.add(i as usize) } as *const u8;
        sum += unsafe { *e } as i64;
        i += 1;
    }
    println!("{}", sum);

    // A byte store writes only its byte: the bytes after it keep their values.
    let mut z: Vec<u8> = vec![9u8; 4];
    let zp = z.as_mut_ptr();
    let z0 = zp as *mut u8;
    unsafe { *z0 = 200; }
    let zr = z.as_ptr();
    let z1 = unsafe { zr.add(1) } as *const u8;
    let z0r = zr as *const u8;
    println!("{}", unsafe { *z0r });
    println!("{}", unsafe { *z1 });

    // `i8` sign-extends: a stored `-5` reads back as `-5`.
    let mut s: Vec<i8> = vec![0i8; 4];
    let sp = s.as_mut_ptr();
    let s0 = sp as *mut i8;
    unsafe { *s0 = -5; }
    let rp = s.as_ptr();
    let r0 = rp as *const i8;
    println!("{}", unsafe { *r0 });

    // A byte pointer cast to a signed one reads the same byte signed: 250 as
    // `i8` is -6.
    let mut c: Vec<u8> = vec![0u8; 4];
    let cp = c.as_mut_ptr();
    let cu = cp as *mut u8;
    unsafe { *cu = 250; }
    let ci = cu as *mut i8;
    println!("{}", unsafe { *ci });

    // A byte value behaves as an `i64`: comparison and arithmetic.
    let cb0 = c.as_ptr();
    let cb = cb0 as *const u8;
    let b = unsafe { *cb };
    println!("{}", b > 100);
    println!("{}", b + 5);

    // A `&mut u8` borrow writes a single byte.
    let mp0 = c.as_mut_ptr();
    let mp = mp0 as *mut u8;
    let mr: &mut u8 = unsafe { &mut *mp };
    *mr = 1;
    let mr2a = c.as_ptr();
    let mr2 = mr2a as *const u8;
    println!("{}", unsafe { *mr2 });

    // A reference to a byte reference nests: `&&u8` and `& &u8` both keep the
    // byte referent, so `&r` (with `r: &u8`) satisfies either spelling.
    let nb = c.as_ptr();
    let nbp = nb as *const u8;
    let nr: &u8 = unsafe { &*nbp };
    let nrr: &&u8 = &nr;
    let nrr2: & &u8 = &nr;
    println!("{}", unsafe { **nrr });
    println!("{}", unsafe { **nrr2 });

    // A byte reference matches a literal pattern: the value is an `i64`, so a
    // byte of `7` matches `7`, and `250` read as `i8` matches `-6`.
    let mut d: Vec<u8> = vec![0u8; 4];
    let dp0 = d.as_mut_ptr();
    let dp = dp0 as *mut u8;
    unsafe { *dp = 7; }
    let dr0 = d.as_ptr();
    let drp = dr0 as *const u8;
    let dr: &u8 = unsafe { &*drp };
    let mv = match dr { 7 => 1, _ => 0 };
    println!("{}", mv);
    let mut e: Vec<u8> = vec![0u8; 4];
    let ep0 = e.as_mut_ptr();
    let ep = ep0 as *mut u8;
    unsafe { *ep = 250; }
    let er0 = e.as_ptr();
    let erp = er0 as *const i8;
    let er: &i8 = unsafe { &*erp };
    let sv = match er { -6 => 10, _ => 20 };
    println!("{}", sv);
}
