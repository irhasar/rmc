use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
static GLOBAL: AtomicU64 = AtomicU64::new(100);
static mut STATIC_ARR: [u64; 8] = [1,2,3,4,5,6,7,8];
thread_local! { static TL: Cell<u64> = const { Cell::new(7) }; }
fn main() {
    let mut total = 0u64;
    std::thread::scope(|s| {
        for t in 0..4 {
            s.spawn(move || {
                for i in 0..1000u64 { TL.with(|c| c.set(c.get().wrapping_add(i))); GLOBAL.fetch_add(i, Ordering::Relaxed); }
                let local = TL.with(|c| c.get());
                println!("thread {} local {} global {}", t, local, GLOBAL.load(Ordering::Relaxed));
            });
        }
    });
    for i in 0..8 { total = total.wrapping_add(unsafe { STATIC_ARR[i] }); }
    println!("total {} global {}", total, GLOBAL.load(Ordering::Relaxed));
}
