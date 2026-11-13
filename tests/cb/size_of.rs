// Regression: `size_of::<T>()` lowers to `core::mem::size_of::<T>()` inside a
// promoted constant through the backend route, which the front end must fold.
fn main() {
    println!("{}", std::mem::size_of::<i64>());
    println!("{}", std::mem::size_of::<u8>());
    println!("{}", std::mem::size_of::<i32>() * 4);
}
