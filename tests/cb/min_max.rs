// Regression: `3.min(5)` / `3.max(5)`. The backend route inlines `Ord::min`
// but leaves `core::intrinsics::integer_min` as a call, which the front end
// must map to the same runtime helper as the un-inlined `<i32 as Ord>::min`
// spelling the prototype sees.
fn main() {
    println!("{}", 3.min(5));
    println!("{}", 3.max(5));
    println!("{}", (-7i64).min(2i64));
    println!("{}", 9u32.max(4u32));
}
