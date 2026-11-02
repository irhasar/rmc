# The rmc language subset

`rmc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 text files with the `.rms` extension. Everything is checked at compile
time; there are no runtime type errors.

## Types

| type         | meaning                                       |
| ------------ | --------------------------------------------- |
| `i64`        | signed 64-bit integer                         |
| `bool`       | `true` or `false`                             |
| `[T; N]`     | fixed-size array of `N` `i64`/`bool` elements |
| `Name`       | a struct declared with `struct Name { .. }`   |
| `()`         | unit, the result of a statement or `main`     |

Integer and boolean literals are inferred. Explicit types can be given on
`let` bindings and function signatures. `as` casts between `i64` and `bool`.

## Structs

```rust
struct Point { x: i64, y: i64 }

let p: Point = Point { x: 3, y: 4 };   // fields may be given in any order
let mut q: Point = Point { x: 0, y: 0 };
q.x = q.x + 1;                          // field read and write
```

Fields are laid out in declaration order with no padding. A struct may contain
another struct, but not itself, and it cannot be used as an array element.
Structs may be passed to and returned from functions by value, assigned to one
another (`p = q`), and nested as fields; in every case the value is copied, so a
change to one does not reach the other. To reuse a
struct after passing it (as `bump(a)` then reading `a`), annotate it
`#[derive(Copy, Clone)]` so the same source also compiles with rustc; rmc parses
and ignores the attribute.

## Functions

```rust
fn name(a: i64, b: bool) -> i64 { ... }
```

- Up to six parameters. Scalars travel in the System V registers
  `rdi rsi rdx rcx r8 r9`; a struct or array is passed by value as a pointer to a
  copy. A returned aggregate is written through a caller-provided buffer.
- The return type may be omitted, in which case it is `()`.
- Recursion is supported. `main` is the entry point; its return value becomes
  the process exit status.
- A function body's last expression is its value, or use `return`.

## Statements

```rust
let x: i64 = 1;          // immutable binding
let mut y: i64 = 0;      // mutable binding
y = y + 1;               // assignment (target may be a variable, array element or field)
y += 1;                  // compound assignment: += -= *= /= %=
x;                       // expression statement
if cond { ... } else { ... }
while cond { ... }
for i in 0..10 { ... }   // i64 range, end exclusive
loop { ... }
break;
continue;
return expr;
```

`let` without an initialiser zero-initialises the binding. `for` bounds are
evaluated once, before the loop; `continue` still runs the increment.

## Overflow

Array indices are `i64` and are unchecked by default, as in C. Compiling with
`--bounds-checks` (or `CompileOptions { bounds_checks: true }`) checks every
index against the array's length; an out-of-range index writes
`rmc: index out of bounds` to stderr and exits with status 101. A constant index
is checked when the program is compiled, not at run time.

By default integer arithmetic wraps, as in release-mode Rust. Compiling with
`--overflow-checks` (or `CompileOptions { overflow_checks: true }` from the
library) inserts a trap after every `+`, `-` and `*`; on overflow the program
writes `rmc: arithmetic overflow` to stderr and exits with status 101, matching a
Rust panic. Division by zero always traps at the CPU level.

## Expressions

- Arithmetic: `+ - * / %` on `i64`.
- Comparison: `== != < <= > >=`, producing `bool`.
- Boolean: `&& || !`, short-circuiting.
- Bitwise: `& | ^` on `i64`, plus `<<` and `>>`. `>>` is arithmetic (a negative
  value stays negative) and a shift count is taken modulo 64, matching Rust.
  Precedence, tightest first: `* / %`, `+ -`, `<< >>`, `&`, `^`, `|`, then the
  comparisons and `&& ||`. So `1 + 2 << 3` is `(1 + 2) << 3`, and
  `if x & 1 == 0` is `(x & 1) == 0`.
- Calls: `f(a, b)`.
- Indexing: `arr[i]`.
- Array literals: `[a, b, c]` and `[value; count]`.
- Struct literals: `Point { x: 1, y: 2 }`, usable as a `let` initialiser.
- Blocks and `if`/`else` are expressions.
- String literals, given type `i64` (a pointer to a `{len, bytes}` record), so
  they can be passed to `print_str`.

## Builtins

Provided by the runtime; no declaration or import is required.

| builtin         | signature      |
| --------------- | -------------- |
| `print_i64(x)`  | `(i64) -> ()`  |
| `print_str(s)`  | `(i64) -> ()`  |
| `print_bool(b)` | `(bool) -> ()` |
| `putchar(c)`    | `(i64) -> ()`  |
| `alloc(n)`      | `(i64) -> i64` |
| `free(p)`       | `(i64) -> ()`  |
| `arg_i64(i)`    | `(i64) -> i64` |

`alloc` returns a pointer to `n` bytes of writable memory, or 0 on failure; it
is a first-fit allocator over an mmap'd arena, and `free` returns a block for
reuse and coalesces neighbours. Memory is not zeroed.

## Not supported

Enums, traits, generics, `match`, references, borrowing,
lifetimes, `impl` blocks, methods, modules, macros, `use`, closures, `f32`/`f64`,
`u*`/`i*` types other than `i64`, tuples, and the standard library. Aggregates
cross a call only by value; there are no `&T`/`&mut T` parameters.
