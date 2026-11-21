use std::collections::{HashMap, BTreeMap, HashSet};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key { a: u64, b: String }

fn sum_generic<T: IntoIterator<Item = u64>>(it: T) -> u64 { it.into_iter().fold(0, u64::wrapping_add) }

trait Shape { fn area(&self) -> f64; fn name(&self) -> &'static str; }
struct Circle(f64); struct Rect(f64, f64); struct Tri(f64, f64);
impl Shape for Circle { fn area(&self)->f64{ 3.14159265358979*self.0*self.0 } fn name(&self)->&'static str{"circle"} }
impl Shape for Rect { fn area(&self)->f64{ self.0*self.1 } fn name(&self)->&'static str{"rect"} }
impl Shape for Tri { fn area(&self)->f64{ 0.5*self.0*self.1 } fn name(&self)->&'static str{"tri"} }

fn main() {
    let mut h: HashMap<Key, u64> = HashMap::new();
    for i in 0..2000u64 { h.insert(Key{a:i, b:format!("k{}", i)}, i.wrapping_mul(2654435761)); }
    let mut acc: u64 = 0;
    for i in 0..2000u64 { if let Some(v) = h.get(&Key{a:i, b:format!("k{}", i)}) { acc = acc.wrapping_add(*v); } }
    let s = sum_generic((0..5000u64).map(|x| x.wrapping_mul(x)));
    let shapes: Vec<Box<dyn Shape>> = vec![Box::new(Circle(2.0)), Box::new(Rect(3.0,4.0)), Box::new(Tri(6.0,7.0))];
    let mut area = 0.0; for sh in &shapes { area += sh.area(); }
    let names: String = shapes.iter().map(|s| s.name()).collect::<Vec<_>>().join(",");
    let mut bt: BTreeMap<u64, u64> = BTreeMap::new();
    for i in 0..1000u64 { bt.insert((i*7919)%1009, i); }
    let bts: u64 = bt.values().copied().fold(0, u64::wrapping_add);
    let mut hs: HashSet<u64> = HashSet::new();
    for i in 0..3000u64 { hs.insert(i.wrapping_mul(31)); }
    let mut v: Vec<u64> = (0..30000u64).map(|x| (x*2654435761)%1000003).collect();
    v.sort_unstable();
    let sorted_sum: u64 = v.iter().step_by(997).copied().fold(0, u64::wrapping_add);
    let mut words: Vec<String> = (0..300).map(|i| format!("word{}-{}", i, i*i)).collect();
    words.sort();
    let joined = words.join("|");
    println!("{} {} {} {} {} {} {} {}", acc, s, area, names, bts, hs.len(), sorted_sum, joined.len());
}
