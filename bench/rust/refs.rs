// The same `&self` methods over a wide struct, called in a hot loop.
//
// Fairness note: the struct's field values come from `black_box(n)`, a
// runtime-opaque seed, so neither compiler can constant-fold the accessor
// sum (LLVM folds a literal struct to a constant and drops the loop). The
// accumulator is reduced by a data-dependent branch, which keeps the loop a
// real per-iteration loop: it defeats LLVM's loop-closing (`n * K`) and its
// vectoriser (`paddq`), so both compilers run a scalar loop of the same shape
// and the ratio reflects the reference/accessor codegen rather than a barrier
// one side pays for and the other does not. There is deliberately no in-loop
// `black_box`: rmc lowers it to the identity while rustc makes it opaque, so an
// in-loop `black_box` would charge rustc a store/reload rmc never emits and
// make the comparison meaningless.
#[derive(Copy, Clone)]
struct Wide { a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64 }

impl Wide {
    fn total(&self) -> i64 {
        self.a + self.b + self.c + self.d + self.e + self.f + self.g + self.h
    }
    fn pair(&self) -> i64 {
        self.a * self.h + self.d * self.e
    }
}

fn sum_via_ref(w: &Wide) -> i64 {
    w.total() + w.pair()
}

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let n = std::hint::black_box(n);
    // Opaque, small field values: the sum is a runtime value, not a constant.
    let s = std::hint::black_box(n) % 1000;
    let w = Wide { a: s, b: s + 1, c: s + 2, d: s + 3, e: s + 4, f: s + 5, g: s + 6, h: s + 7 };
    let wr: &Wide = &w;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        acc += wr.total() + sum_via_ref(wr) + wr.pair();
        if acc > 1000000 {
            acc -= 1000000;
        }
        i += 1;
    }
    println!("{}", acc);
}
