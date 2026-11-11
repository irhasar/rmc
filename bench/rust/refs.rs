// The same `&self` methods over a wide struct,
// called in a hot loop.
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
    let w = Wide { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8 };
    let wr: &Wide = &w;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        // `black_box(n)` keeps the trip count opaque, but the body is otherwise
        // loop-invariant, so LLVM closes the whole loop into `128 * n` and the
        // loop disappears. `black_box` on a term is what stops that: it forces
        // a store/reload per iteration, which is why the Rust loop is not
        // instruction-identical to the rmc one. The rmc side has no `black_box`
        // (it is a compiler, not a benchmark language) and its licm deliberately
        // will not hoist a read through a reference (rmc's `&T` is not known to
        // be dereferenceable, so a speculative load could fault), so the rmc
        // loop reloads all eight fields every iteration. Read the `refs` ratio
        // as "two real loops doing the same arithmetic", not as an
        // instruction-for-instruction tie.
        acc = acc + std::hint::black_box(wr.total()) + sum_via_ref(wr) + wr.pair();
        i += 1;
    }
    println!("{}", acc);
}
