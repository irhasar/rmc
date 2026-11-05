//! heap benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

struct Node {
    val: i64,
    next: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 200000;
    }
    let mut head: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let p: i64 = alc(16);
        let np: &mut Node = unsafe { &mut *(p as *mut Node) };
        np.val = i;
        np.next = head;
        head = p;
        i = i + 1;
    }
    let mut sum: i64 = 0;
    let mut cur: i64 = head;
    while cur != 0 {
        let cp: &Node = unsafe { &*(cur as *const Node) };
        sum = sum + cp.val;
        cur = cp.next;
    }
    print(sum);
    0
}
