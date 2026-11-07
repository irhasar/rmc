// Nested functions: an item declared inside a block. A nested `fn` captures
// nothing, so it is hoisted to the top level and behaves like a free function
// that happens to be spelled inside the block. It sees the program's items
// (a top-level `fn`, a `const`) but no enclosing local.
const K: i64 = 10;

fn top(x: i64) -> i64 {
    return x + 1;
}

fn main() {
    fn sq(x: i64) -> i64 {
        return x * x;
    }
    fn fact(n: i64) -> i64 {
        if n <= 1 {
            return 1;
        }
        return n * fact(n - 1);
    }
    println!("{}", sq(7));
    println!("{}", fact(6));
    // A nested `fn` may call a top-level `fn` and read a top-level `const`.
    fn uses_top(x: i64) -> i64 {
        return top(x) + K;
    }
    println!("{}", uses_top(5));
    // A `fn` declared inside another `fn`'s body.
    fn outer(x: i64) -> i64 {
        fn twice(y: i64) -> i64 {
            return y + y;
        }
        return twice(x) + 1;
    }
    println!("{}", outer(9));
    // A nested `fn` is visible in the whole block, so it may be called before
    // its declaration.
    println!("{}", later(3));
    fn later(z: i64) -> i64 {
        return z * 100;
    }
    // A nested `fn` with an aggregate parameter and a `bool` parameter.
    fn choose(flag: bool, p: (i64, i64)) -> i64 {
        if flag {
            return p.0;
        }
        return p.1;
    }
    println!("{}", choose(true, (4, 5)));
    println!("{}", choose(false, (4, 5)));
    // A nested `fn` shadows a top-level `fn` of the same name inside its block.
    // `top` is the top-level `fn` above (returns `x + 1`); inside `main` the
    // nested `top` (returns `x + 100`) wins.
    fn top(x: i64) -> i64 {
        return x + 100;
    }
    println!("{}", top(5));
    // A `fn` declared in an inner block is scoped to that block and shadows
    // nothing outside it; the outer `top` is visible again after the block.
    {
        fn top(x: i64) -> i64 {
            return x + 1000;
        }
        println!("{}", top(5));
    }
    println!("{}", top(5));
}
