use std::sync::atomic::{AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

fn main() {
    let mut sum: u64 = 0;
    for i in 0..64u64 {
        tick();
        sum += i;
    }
    println!("{} {}", sum, TICKS.load(Ordering::Relaxed));
}
