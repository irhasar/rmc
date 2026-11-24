#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (particles), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Physics simulation: fixed-point particles with gravity and wall bounces,
// an array of structs updated in place (game loop, particle system).
#[derive(Clone, Copy)]
struct P {
    x: i64,
    y: i64,
    vx: i64,
    vy: i64,
}

const NP: usize = 2000;
const LIMIT: i64 = 1000000;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn step(ps: &mut [P; NP]) {
    let mut i: usize = 0;
    while i < NP {
        let mut p = ps[i];
        p.vy -= 3;
        p.x += p.vx;
        p.y += p.vy;
        if p.x < 0 {
            p.x = -p.x;
            p.vx = -p.vx;
        }
        if p.x > LIMIT {
            p.x = 2 * LIMIT - p.x;
            p.vx = -p.vx;
        }
        if p.y < 0 {
            p.y = -p.y;
            p.vy = -p.vy * 9 / 10;
        }
        if p.y > LIMIT {
            p.y = 2 * LIMIT - p.y;
            p.vy = -p.vy;
        }
        ps[i] = p;
        i += 1;
    }
}

fn run(n: i64) {
    let steps = if n == 0 { 20000 } else { n };
    let mut ps: [P; NP] = [P { x: 0, y: 0, vx: 0, vy: 0 }; NP];
    let mut seed: i64 = 4242;
    let mut i: usize = 0;
    while i < NP {
        seed = xorshift(seed);
        let a = (seed >> 1) % LIMIT;
        seed = xorshift(seed);
        let b = (seed >> 1) % LIMIT;
        seed = xorshift(seed);
        let c = (seed >> 1) % 2001 - 1000;
        seed = xorshift(seed);
        let d = (seed >> 1) % 2001 - 1000;
        ps[i] = P { x: a, y: b, vx: c, vy: d };
        i += 1;
    }
    let mut s: i64 = 0;
    while s < steps {
        step(&mut ps);
        s += 1;
    }
    let mut energy: i64 = 0;
    let mut cx: i64 = 0;
    let mut k: usize = 0;
    while k < NP {
        energy += ps[k].vx * ps[k].vx + ps[k].vy * ps[k].vy;
        cx += ps[k].x;
        k += 1;
    }
    out(energy);
    out(cx / (NP as i64));
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
