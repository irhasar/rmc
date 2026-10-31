# The nrc language subset

`nrc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 text files with the `.nrs` extension. Everything is checked at compile
time; there are no runtime type errors.

## Types

| type         | meaning                                   |
| ------------ | ----------------------------------------- |
| `i64`        | signed 64-bit integer                     |
| `bool`       | `true` or `false`                         |
| `[T; N]`     | fixed-size array of `N` elements of `T`   |
| `()`         | unit, the result of a statement or `main` |

Integer and boolean literals are inferred. Explicit types can be given on
`let` bindings and function signatures. `as` casts between `i64` and `bool`.

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
y = y + 1;               // assignment (target may be a variable or array element)
x;                       // expression statement
if cond { ... } else { ... }
while cond { ... }
loop { ... }
break;
continue;
return expr;
```

`let` without an initialiser zero-initialises the binding.

## Expressions

- Arithmetic: `+ - * / %` on `i64`.
- Comparison: `== != < <= > >=`, producing `bool`.
- Boolean: `&& || !`, short-circuiting.
- Calls: `f(a, b)`.
- Indexing: `arr[i]`.
- Array literals: `[value; count]`, usable as a `let` initialiser.
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
| `arg_i64(i)`    | `(i64) -> i64` |

## Not supported

Structs, enums, traits, generics, `match`, `for`, references, borrowing,
lifetimes, `impl` blocks, modules, macros, `use`, closures, `f32`/`f64`, `u*`/`i*`
types other than `i64`, tuples, and the standard library.
