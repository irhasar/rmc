# The rmc language subset

`rmc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 text files with the `.rms` extension. Everything is checked at compile
time; there are no runtime type errors.

## Types

| type         | meaning                                       |
| ------------ | --------------------------------------------- |
| `i64`        | signed 64-bit integer                         |
| `bool`       | `true` or `false`                             |
| `[T; N]`     | fixed-size array of `N` `i64`, `bool` or unit-enum elements |
| `&T` `&mut T`| a reference to a `T` (one word)               |
| `Name`       | a struct declared with `struct Name { .. }`   |
| `Name`       | an enum declared with `enum Name { .. }`      |
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

## Methods

```rust
struct Point { x: i64, y: i64 }

impl Point {
    fn sum(self) -> i64 { return self.x + self.y; }
    fn moved(self, dx: i64) -> Point { return Point { x: self.x + dx, y: self.y }; }
}

let p: Point = Point { x: 3, y: 4 };
let q: Point = p.moved(1);
p.sum();                     // 7
q.sum();                     // 8
```

An `impl Type { .. }` block adds methods to a struct. A method takes `self` as
its first parameter. With `self`, the receiver is passed by value like any other
argument, so the method body works on a private copy and may assign its fields
without affecting the caller. With `&self` (or `&mut self`), the receiver is a
reference: the call passes the value's address instead of a copy, which matters
for a large struct in a hot loop. `Self` names the enclosing struct, in both type
and literal position. The method is resolved from the receiver's type, so two
structs may define a method with the same name. A method call chains
(`p.moved(1).sum()`) and a struct literal may be a receiver directly
(`Point { x: 1, y: 2 }.sum()`).

## Enums

```rust
enum Color { Red, Green, Blue }

let c: Color = Color::Green;
c as i64;                    // 1, the declaration index
```

An enum is a set of **unit variants** (no payload). It is represented by its
`i64` tag, so it costs nothing over an integer and codegen needs no new case for
it. A variant is written `Color::Green`; its value is its declaration index
(0, 1, 2, ...). An enum value may be cast **to** `i64`, compared with `==`/`!=`
against another value of the same enum, stored in a struct field, passed and
returned by value, and used as a `match` scrutinee with `Color::Green` patterns.
An enum may also have `impl` methods. `as` cannot convert an `i64` **to** an
enum (rustc rejects that too); use a `match` to go from a tag back to a variant.

### Enums with a payload

```rust
enum Opt { None, Some(i64) }

let a: Opt = Opt::Some(3);
match a {
    Opt::Some(v) => v,     // binds the payload
    Opt::None => 0,
    _ => 0,                // a catch-all arm, as in Rust
}
```

A variant may carry data. Such an enum is a **tagged union**: `{ i64 tag,
<payload> }`, where the payload is as wide as the widest variant and starts at
byte 8. It is an aggregate like a struct, so it is named by its address, passed
and returned by pointer, and copied word by word on assignment. Its size is
computed by sema and carried in the type, so codegen needs no lookup.

A payload variant is built with `Opt::Some(3)`; a unit variant of a payload enum
is still written `Opt::None`, and is materialised as a zero-payload value.
A payload enum may have `impl` methods like a unit enum; a `self` method takes
the value by pointer (it is an aggregate) and a `&self` method takes its
address.
A pattern that matches a payload variant must bind its fields, `Opt::Some(v)`;
matching it bare (`Opt::Some`) is an error, because there is no way to name the
value. A payload variant is not comparable with `==` and cannot be cast with
`as` (it is not a scalar), and it may not be an array element.

A recursive enum such as `enum L { Nil, Cons(i64, L) }` is rejected: rmc has no
indirection, so an enum may not contain itself.

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

`let` without an initialiser defers initialisation: the binding must be written
before it is read, exactly as in Rust. Reading a binding that is not definitely
initialised on every path is rejected (rustc's `E0381`), so `let x: i64;` then
`return x;` does not compile. Writing one field or element of an uninitialised
aggregate leaves it uninitialised as a whole, so reading it is still an error
(rustc's "partially assigned"). A `let` or parameter without `mut` is
immutable: assigning it (rustc's `E0384`), assigning a field or element of it
(`E0594`), or taking `&mut` of it (`E0596`) is rejected. Add `mut` (or `mut x:
T` on a parameter) to write it. A `&mut T` reference is writable through without
declaring the reference binding itself `mut`, exactly as in Rust. `for` bounds
are evaluated once, before the loop; `continue` still runs the increment.

## Overflow

Array indices are `i64` and are unchecked by default, as in C. Compiling with
`--bounds-checks` (or `CompileOptions { bounds_checks: true }`) checks every
*variable* index against the array's length; an out-of-range index writes
`rmc: index out of bounds` to stderr and exits with status 101. A *constant*
index outside the array (including a negated one, `a[-1]`) is rejected at
compile time whether or not `--bounds-checks` is set, matching rustc.

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
  `if x & 1 == 0` is `(x & 1) == 0`. rmc has only `i64`, so a literal count is
  masked to 6 bits; in Rust an unconstrained literal count is `i32` and masks to
  5, which is why a program that mixes the two types can differ.
- Nesting is bounded: an expression, block or type may nest at most 100 levels.
  Deeper input is a compile error, not a crash. No hand-written program comes
  close; the limit exists so that pathological input cannot overflow the stack.
- References: `&e` takes the address of a place (a variable, a field, an array
  element, or a dereference), or of a temporary when `e` has no address of its
  own; `*e` reads the referent, and `*e = v` writes it. `&mut e` is a distinct
  type from `&e` but otherwise behaves the same, except that a `&mut T` coerces
  to a `&T` where a shared reference is wanted. Reading a field, indexing, or
  calling a method through a reference does not need an explicit `*`: `r.x`,
  `r[i]` and `r.m()` auto-deref. A reference is one word whether it points at a
  scalar or an aggregate, so a `&Point` parameter costs one register and no
  copy. There is no borrow checking: rmc does not track how long a reference
  lives, so it is the programmer's job not to outlive the referent, and `&mut` is
  not enforced to be exclusive.
- Calls: `f(a, b)`.
- Indexing: `arr[i]`.
- Array literals: `[a, b, c]` and `[value; count]`. A list literal takes its
  element type from the values, so `[true, false]` is a `[bool; 2]` and
  `[1, 2, 3]` an `[i64; 3]`; mixing the two is an error. An array element is a
  scalar — `i64`, `bool` or a unit enum — so arrays of arrays, of structs and of
  payload enums are not supported (rustc rejects the last two as well).
- Struct literals: `Point { x: 1, y: 2 }`, usable as a `let` initialiser.
- Blocks and `if`/`else` are expressions.
- `match` on integers or enums: `match x { 0 => a, 1 => b, _ => c }` or
  `match c { Color::Red => a, _ => b }`. An arm pattern is an integer literal
  (optionally negated), an `Enum::Variant`, or `_`. The first equal arm runs and
  `_` matches anything; it is an expression, usable as a statement or a value,
  and a comma before `}` may be omitted. A `match` must be exhaustive, exactly as
  in Rust: an arm must be `_` (or a binding name), or the arms must name *every*
  variant of one enum. A non-exhaustive `match` is rejected with an error even
  when it stands alone as a statement, since rustc rejects it there too.
- String literals, given their own type (a pointer to a `{len, bytes}` record).
  A string is not an `i64`: it cannot be an arithmetic operand and cannot be
  passed where an `i64` is expected. The only thing to do with one is pass it to
  `print_str`.

## Builtins

Provided by the runtime; no declaration or import is required.

| builtin         | signature      |
| --------------- | -------------- |
| `print_i64(x)`  | `(i64) -> ()`  |
| `print_str(s)`  | `(&str) -> ()` |
| `print_bool(b)` | `(bool) -> ()` |
| `putchar(c)`    | `(i64) -> ()`  |
| `alloc(n)`      | `(i64) -> i64` |
| `free(p)`       | `(i64) -> ()`  |
| `arg_i64(i)`    | `(i64) -> i64` |

`alloc` returns a pointer to `n` bytes of writable memory, or 0 on failure; it
is a first-fit allocator over an mmap'd arena, and `free` returns a block for
reuse and coalesces neighbours. Memory is not zeroed.

## Not supported

Traits, generics, borrowing, lifetimes, modules, macros, `use`, closures,
`f32`/`f64`, `u*`/`i*` types other than `i64`, tuples, arrays of aggregates, and
the standard library. References exist (`&T` / `&mut T`, `&e`, `*e`, `*e = v`),
but without borrow checking: rmc does not prove that a reference outlives its
referent, and `&mut` carries no exclusivity guarantee. An aggregate is still
passed by value unless a reference is used.
