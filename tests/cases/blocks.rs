fn main() {
    let x: i64 = 1;
    {
        let x: i64 = 2;
        {
            let x: i64 = 3;
            println!("{}", x);
        }
        println!("{}", x);
    }
    println!("{}", x);
    {
        let y: i64 = 40;
        println!("{}", y);
    }
    println!("{}", 50);
}
