# The rmc language subset

`rmc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 text files with the `.rms` extension. Everything is checked at compile
time; there are no runtime type errors.

## Types

| type         | meaning                                       |
| ------------ | --------------------------------------------- |
| `i64`        | signed 64-bit integer                         |
| `bool`       | `true` or `false`                             |
| `str`        | a string (one word: a pointer to a `{len, bytes}` record) |
| `[T; N]`     | fixed-size array of `N` `i64`, `bool` or unit-enum elements |
| `&T` `&mut T`| a reference to a `T` (one word)               |
| `Name`       | a struct declared with `struct Name { .. }`   |
| `Name`       | an enum declared with `enum Name { .. }`      |
| `()`         | unit, the result of a statement or `main`     |

Integer and boolean literals are inferred. The smallest `i64` value is written
`-9223372036854775808` (or `-0x8000000000000000`); the magnitude alone is out of
range, as in Rust. Explicit types can be given on
`let` bindings and function signatures. `as` converts among `i64`, `bool` and an
enum tag, and casts a `&mut T` to a `&T`. A reference may only be cast to a
reference of the same type, never to a scalar or to `&mut T`. Two casts go
beyond Rust: `i64 as bool` (Rust spells this `x != 0`) and the raw pointer cast
`i64 as &T` / `i64 as &mut T` (used to turn an `alloc` address into a node
pointer).

## Structs

```rust
struct Point { x: i64, y: i64 }

let p: Point = Point { x: 3, y: 4 };   // fields may be given in any order
let x: i64 = 3;
let y: i64 = 4;
let r: Point = Point { x, y };         // field init shorthand: `x` means `x: x`
let s: Point = Point { y: 5, x };      // shorthand and explicit fields mix
let mut q: Point = Point { x: 0, y: 0 };
q.x = q.x + 1;                          // field read and write
```

Fields are laid out in declaration order with no padding. A struct may contain
another struct, but not itself, and it cannot be used as an array element.
Field init shorthand is supported: `Point { x, y }` is `Point { x: x, y: y }`,
and a bare name must resolve to a local of that name. Structs may be passed to
and returned from functions by value, assigned to one
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

## Constants

```rust
const N: i64 = 5;
const M: i64 = N * 2;      // a constant may name one declared later
```

A `const NAME: TYPE = EXPR;` item is folded at compile time to a single value
and substituted at every use, so it has no storage and no codegen. Only `i64`
and `bool` constants are accepted, which is what an array length or a scalar
expression can use; any other type is rejected. A reference to a constant is
resolved wherever a value may appear, including an array length (`[i64; N]`), an
array repeat count (`[x; N]`), a `for` bound, a condition, and a `match`
pattern, where a bare constant name is that value rather than a fresh binding.
A constant may be named before it is declared, as in Rust. Arithmetic in an
initializer that would trap at run time — division or remainder by zero, `i64`
overflow, or a shift by 64 or more — is rejected, as is a cycle between
constants and an initializer that is not a constant expression. Only top-level
`const` items are supported; a `const` inside a function body is rejected.

A local binding may not reuse a constant's name (rustc's `E0530`), whether it is
a `let`, a parameter or a `for` variable. A `const` and a function share a
namespace, so giving both the same name is an error (rustc's `E0428`).

## Statements

```rust
let x: i64 = 1;          // immutable binding
let mut y: i64 = 0;      // mutable binding
y = y + 1;               // assignment (target may be a variable, array element or field)
y += 1;                  // compound assignment: += -= *= /= %=
x;                       // expression statement
if cond { ... } else { ... }
while cond { ... }
while let PAT = expr { ... }  // re-tests expr each iteration; stops on no match
for i in 0..10 { ... }   // i64 range, end exclusive
for i in 0..=10 { ... }  // i64 range, end inclusive
for x in arr { ... }     // array, by value, left to right (x is the element)
loop { ... }
loop { ... break v; }    // `loop` is an expression: `break v` yields `v`
break;
break v;                 // only legal inside a `loop` (rustc's E0571 otherwise)
continue;
'a: loop { ... }         // labelled loops
'a: while cond { ... }
'a: for i in 0..10 { ... }
break 'a;                // leave the loop labelled `'a` (any enclosing one)
break 'a v;              // ... yielding `v`, when `'a` is a `loop`
continue 'a;             // next iteration of the loop labelled `'a`
return expr;             // `;` may be omitted at the end of a block
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
declaring the reference binding itself `mut`, exactly as in Rust. Parameter
names must be unique, and `main` takes no arguments. Constant arithmetic that
would trap — division or remainder by zero, `i64` overflow, or a shift by 64 or
more — is a compile error, matching rustc; variable operands are still checked
at run time only when `--overflow-checks` is on. `for` bounds are evaluated
once, before the loop; `continue` still runs the increment. `for x in arr`
iterates an array by value: the head is evaluated once, the bound is the
array's compile-time length, and `x` is a fresh immutable binding each
iteration. Only a binding or `_` may be the loop variable; `for x in &arr`
(which Rust iterates by reference) is rejected rather than mistyped. `while let` is
`loop { if let PAT = expr { body } else { break; } }`: the scrutinee is
re-evaluated every iteration, and `continue` re-tests it rather than jumping to
the top.

A branch whose type diverges (`return`, or a `loop` with no reachable `break`)
unifies with any other branch's type, as Rust's `!` does, so
`if c { 1 } else { return 2 }` is accepted. The trailing `;` after
`return`/`break`/`continue` may be omitted when it is the last thing in a block,
since those diverge and no expression can follow them.

`loop` is an expression: `break v` makes the loop itself evaluate to `v`, so
`let x: i64 = loop { ... break 5; };` is accepted. Every `break` in the loop
must agree on one type, with `!` unifying with anything; a `loop` with no
reachable `break` is `!` and so unifies with any context. `break v` is only
legal inside a `loop` (not a `while`/`for`), matching rustc's `E0571`, and a
`loop` used as a statement is typed `()`, so `break v` there is an error.

A loop may carry a label (`'a: loop`, `'a: while`, `'a: for`), and `break 'a` /
`continue 'a` target the innermost enclosing loop with that label rather than the
innermost loop overall, so `break 'a` can leave several nested loops at once.
`break 'a v` yields `v` from a labelled `loop` exactly as `break v` does. A label
that names no enclosing loop is rejected (rustc's `E0426`). A label on `while
let` is rejected rather than silently dropped, because `while let` desugars to a
plain `loop` and the label would be lost.

## Overflow

Array indices are `i64` and are unchecked by default, as in C. Compiling with
`--bounds-checks` (or `CompileOptions { bounds_checks: true }`) checks every
*variable* index against the array's length; an out-of-range index writes
`rmc: index out of bounds` to stderr and exits with status 101. A *constant*
index outside the array (including a negated one, `a[-1]`, and one held in a
constant local, `let i = -1; a[i]`) is rejected at compile time whether or not
`--bounds-checks` is set, matching rustc. A local counts as constant only when
it is assigned at most once and never taken by `&mut`, the same rule rustc uses,
so `let mut i = -1; i = 0; a[i]` stays a runtime (checked) index.

By default integer arithmetic wraps, as in release-mode Rust. Compiling with
`--overflow-checks` (or `CompileOptions { overflow_checks: true }` from the
library) inserts a trap after every `+`, `-` and `*`, and range-checks every
*variable* shift count against `0..64`; on overflow or an out-of-range shift the
program writes `rmc: arithmetic overflow` to stderr and exits with status 101,
matching a Rust panic. Division or remainder by zero, and `i64::MIN / -1` (whose
quotient has no `i64` value), trap the same way whether or not checks are on,
because Rust panics on them in every build mode; a shift count that is a
compile-time constant is decided at compile time, not by the trap.

## Expressions

- Arithmetic: `+ - * / %` on `i64`.
- Comparison: `== != < <= > >=`, producing `bool`.
- Boolean: `&& || !`, short-circuiting. `!` is logical not on a `bool` and
  bitwise not on an `i64`, as in Rust; it yields its operand's type.
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
- `match` on integers, enums or strings: `match x { 0 => a, 1 => b, _ => c }`,
  `match c { Color::Red => a, _ => b }` or `match s { "on" => a, _ => b }`. An
  arm pattern is an integer literal (optionally negated), an integer *range*
  `lo..hi` / `lo..=hi`, a *string* literal, an `Enum::Variant`, an *or-pattern*
  `a | b | c`, or `_`. The first matching arm
  runs and `_` matches anything; it is an expression, usable as a statement or a
  value, and a comma before `}` may be omitted. An arm body is an expression,
  but a bare assignment (`0 => acc = 1`), a compound assignment
  (`1 => acc += 10`), or a `return`/`break`/`continue` is accepted as well, as
  in Rust; a block-bodied arm (`_ => {}`) also works.
- Guards: an arm may carry a condition, `PAT if COND => body`. `COND` is any
  `bool` expression, parsed like an `if` condition, and may read the pattern's
  bindings (`Opt::Some(v) if v > 10 => ..`). The arm runs only when the pattern
  matches *and* the guard holds; otherwise the following arms are tried, so a
  guard is evaluated at most once, in order, and only after its pattern
  matched. A guarded arm covers **nothing** for exhaustiveness — the guard may
  be false — so `match x { n if n > 3 => 1 }` is non-exhaustive even though `n`
  binds every value, and `match e { E::A if c => 1, E::B => 2 }` still needs a
  `_` or a `E::A` arm without a guard, exactly as rustc's E0004 requires.
- A `match` must be exhaustive,
  exactly as in Rust: an arm must be `_` (or a binding name), or the arms must
  name *every* variant of one enum (including across or-patterns, so
  `Color::Red | Color::Green => .., Color::Blue => ..` is exhaustive). A
  non-exhaustive `match` is rejected with an error even when it stands alone as
  a statement, since rustc rejects it there too.
- Or-patterns: `a | b | c` matches when any alternative matches, and the arm
  body runs once. Alternatives may be integers, ranges, `Enum::Variant`s, or
  `_` (an or-pattern containing `_` is irrefutable). A *binding* alternative
  (`x | 1`) is rejected, as rustc's `E0408` rejects it, because rmc does not
  check that the name is bound in every alternative.
- Range patterns: `lo..hi` is half-open (`lo <= v && v < hi`) and `lo..=hi`
  inclusive. Both bounds are `i64` and either may be negative; a range whose
  bounds are out of order (`5..5`, `6..5`) is an empty range and a compile
  error, as in rustc. A range is not by itself exhaustive, so a `_` arm is
  still needed unless the other arms cover every value.
- String patterns: `"lit"` matches a `str` scrutinee by *content*, not by
  pointer. A `str` value is the address of its `{len, bytes}` record, so a
  machine compare of two equal literals at different addresses would answer
  "not equal"; the pattern test is instead the same byte-wise comparison
  `==` uses, and so calls the runtime helper. A string pattern is never
  exhaustive on its own, so a `_` (or binding) arm is still required.
- `if let PAT = EXPR { .. } else { .. }`, sugar for
  `match EXPR { PAT => then, _ => else_or_unit }`. It reuses the `match` pattern
  grammar (including or-patterns and ranges, `if let 1..=5 = x`), but unlike
  `match` it need not be exhaustive: a non-matching value
  falls through to the `_` arm. An `else` is optional; with none and a non-unit
  `then`, the branch types differ and the program is rejected, exactly as rustc
  rejects it — including when the pattern is irrefutable (`if let y = o { 1 }`
  with no `else` is an error). `else if let` chains, and `if let` in value
  position, work like `if`.
- String literals, given their own type (a pointer to a `{len, bytes}` record).
  A string is not an `i64`: it cannot be an arithmetic operand and cannot be
  passed where an `i64` is expected. Two strings may be compared with `==`,
  `!=`, `<`, `<=`, `>` and `>=`, which compare their bytes, not their addresses;
  `print_str` writes one to standard output. `s.len()` gives the byte length:
  it reads the record's length word (a runtime helper, since a `str` is a
  pointer), and is an `i64` where rustc's is a `usize`. `a.len()` on an array of
  length `N` is the compile-time constant `N`, with no runtime work.
- The type of a string literal is named `str` and may be written wherever a type
  goes: a `let` annotation, a parameter, a return type or a struct field
  (`fn f(s: str) -> str`, `struct M { s: str, n: i64 }`). A `str` is one word, so
  a `str` parameter is passed by value as its pointer, and a `str` field is
  copied as one word — unlike Rust's `&str`, which is a two-word fat pointer.

## Builtins

Provided by the runtime; no declaration or import is required.

| builtin         | signature      |
| --------------- | -------------- |
| `print_i64(x)`  | `(i64) -> ()`  |
| `print_str(s)`  | `(str) -> ()`  |
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
