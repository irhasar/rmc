# The nrc language subset

`nrc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 text files with the `.nrs` extension. Everything is checked at compile
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

Fields are laid out in declaration order with no padding. A struct lives in
memory, so it cannot be a function parameter or return value, and it cannot be
used as an array element. A struct may contain another struct, but not itself.

## Functions

```rust
fn name(a: i64, b: bool) -> i64 { ... }
```

- Up to six parameters, passed in the System V registers `rdi rsi rdx rcx r8 r9`.
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

By default integer arithmetic wraps, as in release-mode Rust. Compiling with
`--overflow-checks` (or `CompileOptions { overflow_checks: true }` from the
library) inserts a trap after every `+`, `-` and `*`; on overflow the program
writes `nrc: arithmetic overflow` to stderr and exits with status 101, matching a
Rust panic. Division by zero always traps at the CPU level.

## Expressions

- Arithmetic: `+ - * / %` on `i64`.
- Comparison: `== != < <= > >=`, producing `bool`.
- Boolean: `&& || !`, short-circuiting.
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
`u*`/`i*` types other than `i64`, tuples, and the standard library. Structs
cannot be passed to or returned from functions.
