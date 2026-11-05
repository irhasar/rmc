fn main() {
    let t = std::fs::read_to_string("/tmp/fibn.mir").unwrap();
    let p = rmc::mir::parse(&t).unwrap();
    for f in &p.funcs {
        println!("fn {}", f.name);
        for (i, l) in f.locals.iter().enumerate() {
            println!("  _{i}: ty={:?} is_arg={} name={:?}", l.ty, l.is_arg, l.name);
        }
    }
}
