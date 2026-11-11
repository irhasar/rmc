// A method evaluates and copies its scalar receiver before its arguments, so an
// argument that writes the receiver's slot must not change the value the method
// sees. Each method below is called on `x` with an argument that sets `x`, and
// the printed receiver shows the argument's write did take effect.
fn main() {
    let mut x: i64 = 5;
    let a = x.min({ x = 9; 7 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.max({ x = 9; 7 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.clamp({ x = 9; 0 }, { x = 9; 9 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.midpoint({ x = 9; 1 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.rotate_left({ x = 9; 1 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.rotate_right({ x = 9; 1 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.wrapping_mul({ x = 9; 2 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.div_euclid({ x = 9; 2 });
    println!("{} {}", a, x);

    let mut x: i64 = 5;
    let a = x.rem_euclid({ x = 9; 2 });
    println!("{} {}", a, x);
}
