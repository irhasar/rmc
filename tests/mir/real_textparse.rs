#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (textparse), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Text processing: generate a CSV-like byte buffer of decimal numbers, then
// scan and parse it repeatedly, the shape of a log/CSV parser.
const CAP: usize = 1048576;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn generate(buf: &mut [u8; CAP], seed0: i64) -> i64 {
    let mut seed = seed0;
    let mut len: i64 = 0;
    let mut field: i64 = 0;
    let mut tmp: [u8; 24] = [0; 24];
    while len < (CAP as i64) - 32 {
        seed = xorshift(seed);
        let mut v = (seed >> 1) & 1048575;
        let mut t: i64 = 0;
        if v == 0 {
            tmp[0] = b'0';
            t = 1;
        }
        while v > 0 {
            tmp[t as usize] = b'0' + (v % 10) as u8;
            v /= 10;
            t += 1;
        }
        while t > 0 {
            t -= 1;
            buf[len as usize] = tmp[t as usize];
            len += 1;
        }
        field += 1;
        if field % 10 == 0 {
            buf[len as usize] = b'\n';
        } else {
            buf[len as usize] = b',';
        }
        len += 1;
    }
    len
}

fn parse(buf: &[u8; CAP], len: i64) -> i64 {
    let mut i: i64 = 0;
    let mut cur: i64 = 0;
    let mut sum: i64 = 0;
    let mut lines: i64 = 0;
    let mut maxv: i64 = 0;
    while i < len {
        let c = buf[i as usize];
        if c >= b'0' && c <= b'9' {
            cur = cur * 10 + (c - b'0') as i64;
        } else {
            sum += cur;
            if cur > maxv {
                maxv = cur;
            }
            cur = 0;
            if c == b'\n' {
                lines += 1;
            }
        }
        i += 1;
    }
    sum + lines * 1000003 + maxv
}

fn run(n: i64) {
    let rounds = if n == 0 { 300 } else { n };
    let mut buf: [u8; CAP] = [0; CAP];
    let len = generate(&mut buf, 777777);
    let mut check: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        check = check.wrapping_mul(7).wrapping_add(parse(&buf, len - (r % 7)));
        r += 1;
    }
    out(len);
    out(check);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
