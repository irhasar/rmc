#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (bfs), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Graph search: breadth-first search over a random grid maze, the shape of
// path-finding in a game, a router or a dependency resolver.
const W: i64 = 300;
const H: i64 = 300;
const CELLS: usize = 90000;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn run(n: i64) {
    let rounds = if n == 0 { 60 } else { n };
    let mut wall: [i64; CELLS] = [0; CELLS];
    let mut dist: [i64; CELLS] = [0; CELLS];
    let mut queue: [i64; CELLS] = [0; CELLS];
    let mut seed: i64 = 1234567;
    let mut sum_all: i64 = 0;
    let mut reach_all: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        let mut c: usize = 0;
        while c < CELLS {
            seed = xorshift(seed);
            wall[c] = if (seed >> 1) % 100 < 28 { 1 } else { 0 };
            dist[c] = -1;
            c += 1;
        }
        wall[0] = 0;
        dist[0] = 0;
        queue[0] = 0;
        let mut head: usize = 0;
        let mut tail: usize = 1;
        while head < tail {
            let cur = queue[head];
            head += 1;
            let x = cur % W;
            let y = cur / W;
            let d = dist[cur as usize] + 1;
            if x > 0 {
                let nb = cur - 1;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if x < W - 1 {
                let nb = cur + 1;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if y > 0 {
                let nb = cur - W;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if y < H - 1 {
                let nb = cur + W;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
        }
        let mut k: usize = 0;
        while k < CELLS {
            if dist[k] > 0 {
                sum_all += dist[k];
            }
            k += 1;
        }
        reach_all += tail as i64;
        r += 1;
    }
    out(reach_all);
    out(sum_all);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
