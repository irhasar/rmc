# The rmc language subset

`rmc` accepts a small, statically typed subset of Rust. Programs are ordinary
UTF-8 `.rs` source files, the same ones `rustc` compiles; rmc can stand in for
`rustc` on the subset it supports. Everything is checked at compile time; there
are no runtime type errors.

## Types

| type         | meaning                                       |
| ------------ | --------------------------------------------- |
| `i64`        | signed 64-bit integer                         |
| `bool`       | `true` or `false`                             |
| `char`       | a Unicode scalar value (a 4-byte code point, held in an 8-byte slot) |
| `str`        | a string (one word: a pointer to a `{len, bytes}` record) |
| `[T; N]`     | fixed-size array of `N` `T` elements, where `T` is a scalar, a struct or another array |
| `(A, B)`     | a tuple of the given element types (an aggregate) |
| `&T` `&mut T`| a reference to a `T` (one word)               |
| `Name`       | a struct declared with `struct Name { .. }`   |
| `Name`       | an enum declared with `enum Name { .. }`      |
| `Name`       | a `type` alias declared with `type Name = T;` |
| `()`         | unit, the result of a statement or `main`     |

Integer, boolean and character literals are inferred. An integer literal may
carry a width suffix (`200u8`, `1i64`, `0xffu32`, `100usize`); the value is typed
at that width, so `200u8 + 100` is an overflow (as in Rust) and a magnitude that
does not fit (`300u8`, `128i8`) is a compile error. A suffix naming the native
64-bit integer (`i64`/`u64`/`isize`/`usize`) is a plain `i64`. The smallest `i64` value
is written `-9223372036854775808` (or `-0x8000000000000000`); the magnitude
alone is out of range, as in Rust. A `char` literal is a single Unicode scalar
in single quotes, with the usual escapes (`\n`, `\t`, `\r`, `\0`, `\'`, `\\`,
`\xNN` restricted to `\x00`-`\x7f`, and `\u{..}`), and a non-ASCII character may
also be written directly. A byte literal `b'A'` is an integer (its byte value,
`0`-`255`); rmc has no `u8`, so it is an `i64`, and its `\x` escape may use the
full `\x00`-`\xff` range. A byte *string* `b".."` is not supported. Explicit
types can be given on
`let` bindings and function signatures. `as` converts among `i64`, `bool` and an
enum tag, casts a `char` to its `i64` scalar value, and casts a `&mut T` to a
`&T`. A reference may only be cast to
a reference of the same type, never to a scalar or to `&mut T`. Two casts go
beyond Rust: `i64 as bool` (Rust spells this `x != 0`) and the raw pointer cast
`i64 as &T` / `i64 as &mut T` (used to turn an `alloc` address into a node
pointer). `i64 as char` is rejected, matching Rust, which allows only `u8 as
char`.

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

let base: Point = Point { x: 1, y: 2 };
let u: Point = Point { x: 9, ..base }; // struct update: y is copied from base
```

Struct update syntax `Point { field: value, ..base }` fills every field not
written above the base from `base`. The written field values are evaluated first,
left to right, then the base — the order rustc uses — so a field value with a
side effect runs before the base. The result is a fresh value, so mutating it
does not change `base`. `Point { ..base }` with no written fields is allowed, as
is a struct literal as the base (`Point { x: 3, ..Point { y: 4, ..base } }`); a
comma may not follow the base.

Fields are laid out in declaration order with no padding. A struct may contain
another struct or an array, but not itself. Field init shorthand is supported: `Point { x, y }` is `Point { x: x, y: y }`,
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
    fn new(x: i64, y: i64) -> Point { return Point { x, y }; }  // associated fn
    fn sum(self) -> i64 { return self.x + self.y; }
    fn moved(self, dx: i64) -> Point { return Point { x: self.x + dx, y: self.y }; }
}

let p: Point = Point::new(3, 4);
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

A function in an `impl` block that does **not** take `self` is an **associated
function**: it is called through the type, `Point::new(3, 4)`, not on a value.
This is the usual constructor idiom (`String::new`, `Vec::with_capacity`), and
it works on enums too (`Tag::from_flag(true)`). An associated function is
resolved to the mangled free function `Type$name` the same way a method is; the
only difference is that no receiver is inserted as the first argument. Calling
one with `.` (`p.new(1, 2)`) is therefore an arity error — the receiver would be
an extra argument — and naming one without a call (`let f = Point::new;`) is
rejected, since rmc has no function values. A name in `Type::name` that is not a
variant of that enum and not an associated function is reported as such.

### Built-in integer methods

An `i64` receiver has twenty methods built in, which desugar to the operators
rmc already lowers (so they cost no call, except `clamp`, for its panic, and the
ten bit methods below, for their per-bit work):

| call                    | meaning                                        |
| ----------------------- | ---------------------------------------------- |
| `x.abs()`               | `if x < 0 { -x } else { x }`                    |
| `x.signum()`            | `-1`, `0` or `1`                                |
| `x.min(y)`              | the smaller of `x` and `y` (returns `x` on a tie) |
| `x.max(y)`              | the larger of `x` and `y` (returns `x` on a tie) |
| `x.clamp(lo, hi)`       | `x` bounded to `[lo, hi]`                       |
| `x.is_positive()`       | `x > 0` (a `bool`)                              |
| `x.is_negative()`       | `x < 0` (a `bool`)                              |
| `x.div_euclid(y)`       | `x / y` rounded toward the mathematical quotient |
| `x.rem_euclid(y)`       | the remainder in `[0, abs(y))`                  |
| `x.midpoint(y)`         | the mean of `x` and `y`, rounded toward zero    |
| `x.count_ones()`        | the number of set bits                          |
| `x.count_zeros()`       | the number of clear bits                        |
| `x.leading_zeros()`     | the zeros above the top set bit (64 when `x` is 0) |
| `x.trailing_zeros()`    | the zeros below the low set bit (64 when `x` is 0) |
| `x.leading_ones()`      | the ones above the low zero bit (0 when `x` is -1) |
| `x.trailing_ones()`     | the ones below the high zero bit (0 when `x` is -1) |
| `x.swap_bytes()`        | the byte order reversed                         |
| `x.reverse_bits()`      | the bit order reversed                          |
| `x.rotate_left(n)`      | the bits rotated left by `n % 64`               |
| `x.rotate_right(n)`     | the bits rotated right by `n % 64`              |

The semantics match `rustc -O`: `i64::MIN.abs()` is `i64::MIN` (it does not
trap; with `--overflow-checks` it traps, as in a debug build), and `signum` is
computed with comparisons only, so `i64::MIN.signum()` is `-1` without overflow.
`is_positive`/`is_negative` return `bool` — unlike the eighteen arithmetic and
bit methods, which return `i64` — and `0` is neither positive nor negative, so
both are `false` there.
`div_euclid`/`rem_euclid` compute `q = x / y` and `r = x % y` and round toward
the mathematical quotient: when `r` is negative, `q` moves one step further from
zero and `r` gains `abs(y)`. They reuse `/` and `%`, so a zero divisor and
`i64::MIN.div_euclid(-1)` panic at run time exactly as the operators do.
`midpoint` is the mean rounded toward zero — `(x + y) / 2` with the sum taken in
unbounded precision, so it does not overflow: `i64::MAX.midpoint(i64::MAX)` is
`i64::MAX` and `i64::MIN.midpoint(i64::MAX)` is `0`. It lowers to
`(x & y) + ((x ^ y) >> 1)` with a one-step correction for an odd negative sum,
which is exactly `rustc`'s definition and never overflows, so the result is the
same with and without `--overflow-checks`.
`clamp` returns `lo`, `x` or `hi`, and panics when `lo > hi` (a message and exit
101, like rustc); it is the one built-in whose panic is a runtime call, so a
program that never calls it does not carry the helper.
The bit methods each lower to a small runtime helper rather than to operators:
`x & (x - 1)` would overflow for `i64::MIN`, whereas a loop over the 64 bits is
exact for every input, so the result is the same with and without
`--overflow-checks`. `swap_bytes` is a single `bswap`; `reverse_bits` is a
per-bit loop; the rotates are a single `rol`/`ror`, whose hardware `% 64` count
mask is exactly Rust's. `leading_ones`/`trailing_ones`/`count_zeros` reuse the
counts on `!x` (`leading_ones(x)` is `leading_zeros(!x)`, `count_zeros(x)` is
`count_ones(!x)`), so they need no helper of their own.
Like `clamp`, each helper is linked only when its method is used.
Each method evaluates its receiver — and, for the two-argument methods, its
arguments — exactly once, so `g().abs()` calls `g` once. A method of the same
name on a struct or enum is unaffected — the built-in only fires when the
receiver's type is `i64`.

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

A variant may write its value explicitly with `= N`, and the value need not be
its index:

```rust
enum Code { Ok = 0, Warn = 4, Err }   // Err is 5, the previous value plus one

Code::Warn as i64;                    // 4, the written value
```

A variant with no `= N` takes the previous variant's value plus one, starting at
0. A `match` arm and an `as i64` both read the written value. Only a unit
variant may have a discriminant — a payload variant's tag is an implementation
detail, so `Variant(i64) = 5` is rejected, as in rustc. Two variants may not
share a value.

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
value. An argument may be a wildcard (`Opt::Some(_)`), a `mut` binding
(`Opt::Some(mut v)`), a nested struct or array sub-pattern (`E::A(S { x })`,
`E::A([a, b])`), or a trailing `..` that ignores the remaining fields
(`E::A(x, ..)`, `E::A(..)`). Each argument is the payload read at its field
index, fed to the same sub-pattern expansion a `let` destructuring uses, so the
naming directions compose. An argument must be irrefutable — a literal
(`E::A(1)`) would need a per-field test rmc does not generate, so it is
rejected, as is `mut` on a nested pattern. Naming more fields than the variant
has, or fewer without a `..`, is an error.
A pattern reads through a reference to the scrutinee, as Rust's match ergonomics
do: `match &e { Opt::Some(v) => .. }` dereferences `&E` to the `E` underneath, so
the test and every payload read go through the one deref (a struct pattern
auto-derefs a `&S` scrutinee in the same way, and so does a literal, range or
bool pattern — `match &x { 5 => .. }`). The referent is what the pattern
is checked against, so a variant of a different enum is still rejected.
A payload variant is not comparable with `==` and cannot be cast with
`as` (it is not a scalar), and it may not be an array element.

A recursive enum such as `enum L { Nil, Cons(i64, L) }` is rejected: rmc has no
indirection, so an enum may not contain itself.

## Tuples

```rust
fn swap(p: (i64, i64)) -> (i64, i64) { return (p.1, p.0); }

let t: (i64, bool) = (1, true);
let a: i64 = t.0;               // element access by position
let (x, y) = swap(t);           // destructuring
let (_, y) = swap(t);           // `_` ignores an element
let (mut x, y) = swap(t);       // a `mut` element is reassignable
```

A tuple is an aggregate: it is named by its address, passed to and returned from
a function by pointer, and may be a struct field, an enum payload, an array
element, or the referent of a reference, exactly like a struct. Elements are laid
out in order with no padding; `()` is unit (no storage) and `(T)` is just `T`, so
a one-element tuple must be written `(T,)` with a trailing comma, as in Rust.
Tuples nest, and `t.0.1` reaches into a nested tuple.

Element access `.0`, `.1`, ... requires the base to be a tuple (or a reference
to one, which auto-derefs); an index past the end is rejected. A tuple may be
destructured in a `let`, a `for` head, a function parameter or a `match` arm,
with `_` to ignore an element and `mut` on a binding. A tuple pattern's element
count must match the tuple's arity exactly, and a tuple pattern never matches an
array even when the counts agree: `let (x,) = (1, 2);` and `let (_, _) = [1, 2];`
are both rejected, as `rustc` rejects them (E0308).

A destructuring pattern may shadow the very variable it reads, and every element
read still sees the original value, as in `rustc`: the expansion copies the
scrutinee before it binds, so `let (x, y) = x;` binds `x` and `y` from the old
`x`.

In a `match` arm or an `if let` / `while let` scrutinee a tuple pattern may be
*refutable*: an element may be a scalar test (an integer, `true`/`false`, a
string, a range, or an or-pattern of those), and the arm runs only when every
element matches:

```rust
match (1, 2) {
    (0, _) => 0,            // the first element must be 0
    (a, b) => a * 10 + b,   // binds both elements
}
if let (7, x) = (7, 8) { print_i64(x); }
while let (0, x) = (0, k) { ... }
```

A refutable element may not bind (rmc generates no test that would carry a
payload), so `(E::V(x), y)` is rejected. A `let`, a `for` head and a parameter
are irrefutable, so a literal element there is rejected (`let (1, x) = e;`).

## Functions

```rust
fn name(a: i64, b: bool) -> i64 { ... }
```

- Up to six parameters. Scalars travel in the System V registers
  `rdi rsi rdx rcx r8 r9`; a struct or array is passed by value as a pointer to a
  copy. A returned aggregate is written through a caller-provided buffer.
- A parameter may be written as an aggregate pattern, `fn f(S { x, y }: S)` or
  `fn f([a, b]: [i64; 2])`, including a nested one (`S { a: [x, y], .. }`). The
  parameter itself is an ordinary argument of the annotation's type; the pattern
  binds its names at the top of the body, exactly as the matching `let`
  destructuring would, so `mut` is per binding and a name may not collide with
  another parameter (rustc's `E0415`). A refutable pattern (a literal, a variant,
  an or-pattern) is rejected, as is `mut` on the whole pattern. An at-binding is
  accepted here too (`fn f(n @ _: i64)`, `fn f(t @ (a, b): (i64, i64))`), with
  `mut` allowed before the name; its sub-pattern must be irrefutable.
- The return type may be omitted, in which case it is `()`.
- Recursion is supported. `main` is the entry point; its return value becomes
  the process exit status.
- A function body's last expression is its value, or use `return`.
- A `fn` may be declared inside a block, including another `fn`'s body. It is an
  item, not a closure: it captures no locals, so it sees only the program's
  items (a top-level `fn`, `struct`, `enum`, `const` or `type`), never an enclosing
  binding. Reading an enclosing local from it is rejected (rustc's `E0434`). A
  nested `fn` is visible in its whole block, so it may be called before its own
  declaration and a sibling may call a `fn` declared later. It shadows a
  top-level `fn` of the same name inside that block, exactly as rustc's scoping
  does; two `fn`s with the same name in one block is an error (rustc's `E0428`).
  Because it captures nothing, a nested `fn` is hoisted to the top level and
  compiled like any other function, so it costs nothing over a free function.

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
constants and an initializer that is not a constant expression.

A `const` may also be declared inside a block, where it is folded and in scope
for the rest of that block. Like a top-level `const` it may be named before it is
declared (rustc hoists block items), and it shadows an outer constant of the same
name until the block ends, when the outer value returns. The whole block's
constants are resolved together, so a forward reference (`const A = B + 1; const
B = 2;`) works and a self-reference (`const K = K + 1;`, where an outer `K`
exists) is a cycle, not a read of the outer value. Every block constant is
validated even when unused, so `const N: i64 = true;` is rejected as it would be
at the top level. Two `const`s of the same name in one block is an error, and so
is a `const` and a `fn` of the same name (rustc's `E0428`).

```rust
fn main() {
    let a = N;             // usable before its declaration
    const N: i64 = 5;
    let b = { const N: i64 = 9; N };   // an inner shadow, 9
    // here `N` is 5 again
    const A: i64 = B + 1;  // a forward reference
    const B: i64 = 2;      // A folds to 3
}
```

A local binding may not reuse a constant's name (rustc's `E0530`), whether it is
a `let`, a parameter, a `for` variable, an `@` binding or a struct-pattern
field. The check covers a block-scoped constant too, not only a top-level one. A
`const` and a function share a namespace, so giving both the same name is an
error (rustc's `E0428`).

An immutable `static NAME: TYPE = EXPR;` (optionally `pub`) is lowered to a
`const`: rmc has no mutable global storage, so a `static` has the same fold-and-
substitute semantics and the same restrictions. A block-scoped `static` is folded
the same way. `static mut` is rejected, since rmc cannot represent a mutable
global and a program that mutated one would diverge.

## Type aliases

```rust
type Count = i64;
type Point = (i64, i64);
type Grid = [[i64; 4]; 4];
struct Node { next: i64 }
type List = Node;

let n: Count = 5;             // a scalar alias
let p: Point = (1, 2);        // an alias for a tuple
let g: Grid = [[0; 4]; 4];    // an alias for a nested array
let l: List = Node { next: 0 };
```

A `type NAME = T;` item gives a second name to a type. The alias is transparent:
it is rewritten to its target when the type is parsed, so it never reaches layout
or code generation, an alias and its target are the same type (a `Count` and an
`i64` may be used interchangeably), and a struct alias is the struct itself, with
the same fields and the same layout. Any type may be aliased, including a
struct, an enum, a tuple, an array, a reference or a scalar.

An alias may be named before it is declared, as in Rust, and one alias may name
another (`type A = B; type B = i64;`) in either declaration order. A cycle
(`type A = B; type B = A;`) is an error (rustc's `E0391`), not a hang. `type` is
a keyword, so it cannot be used as a binding or function name. An alias shares
the type namespace with a struct and an enum, so reusing one of their names (or
declaring the same alias twice) is an error (rustc's `E0428`); a `const` or a
`fn` of the same name is fine, since those live in the value namespace. Only
top-level `type` items are supported.

## Statements

```rust
let x: i64 = 1;          // immutable binding
let mut y: i64 = 0;      // mutable binding
let S { a, b: c, .. } = s;  // struct destructuring: bind `a` and `c`
let S { mut a, b }: S = s;  // per-binding `mut`, optional annotation
let [a, b] = arr;        // array destructuring: bind `a` = arr[0], `b` = arr[1]
let [x, _, .., z] = arr; // `_` ignores an element, `..` absorbs the middle
let [mut a, b] = arr;    // per-element `mut`, optional annotation
let S { a: [x, y], b } = s; // array-typed field destructured in place
let _ = e;               // discard: evaluate `e` for side effects, bind nothing
let _: T;                // annotated discard, no initialiser (a no-op)
y = y + 1;               // assignment (target may be a variable, array element or field)
y += 1;                  // compound assignment: += -= *= /= %=
x;                       // expression statement
;                        // empty statement (any number, anywhere)
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
(rustc's "partially assigned"). `let _ = e;` discards `e`: it is evaluated once
for its side effects and binds nothing, and any annotation is still checked
(`let _: bool = 5;` is rejected, rustc's `E0308`). `let _;` with no annotation
has no type to infer and is rejected (rustc's `E0282`); `let _: T;` is a no-op.
A `let S { .. } = e;` destructures a struct: each named field is bound as a
fresh local in the enclosing block, so it stays visible for the rest of the
block. The scrutinee is evaluated exactly once, and a type annotation after the
pattern (`let S { x, y }: S = e;`) is checked against it. `mut` is per binding —
`let S { mut x, y: mut z } = e;` is accepted, `let mut S { .. } = e;` is
rejected (`mut` must be attached to each individual binding, as in rustc).
A struct-typed field may be destructured by a nested sub-pattern
(`let S { a: I { x } } = e;`), and an array-typed field by an array pattern
(`let S { a: [x, y], b } = e;`); see Struct patterns for the full field grammar.
A `let [a, b, .., c] = e;` destructures an array the same way: each named element
is bound as a fresh local in the enclosing block, the scrutinee is evaluated
exactly once, and a type annotation after the pattern (`let [x, y]: [i64; 2] =
e;`) is checked against it. The expansion is one index read per element, so
`let [a, b] = e;` lowers to exactly `let a = e[0]; let b = e[1];`. A `_` element
binds nothing, `mut` is per element (`let [mut a, b] = e;`), and a single `..`
absorbs any number of middle elements — its trailing bindings read relative to
the array's length, so `let [x, .., z] = a;` binds `a[0]` and `a[a.len() - 1]`
(the latter folds to the constant length). Elements may be nested array or
struct patterns to any depth. An element pattern must be irrefutable, so a
literal element (`let [1, y] = a;`) is rejected rather than mis-bound; a
two-`..` pattern, an out-of-range fixed index, and `mut` on a non-binding are
rejected too. An array pattern must name exactly the array's length (a `..`
rest may absorb the remainder), as rustc requires. Array patterns are also
accepted in `match`, `if let` and `while
let` arms, and as a `for` head (`for [x, y] in pairs`), with the same grammar.
A `let` or parameter without `mut` is
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
iteration. The loop variable may be a binding, `_`, a struct pattern
(`for S { f, .. } in arr`), an array pattern (`for [x, y] in pairs`), or an
at-binding (`for n @ _ in arr`, `for i @ j in 0..3`), which binds
each element's fields or elements fresh each iteration; any other pattern is
refutable and rejected. `for x in &arr`
(which Rust iterates by reference) is rejected rather than mistyped. `while let` is
`loop { if let PAT = expr { body } else { break; } }`: the scrutinee is
re-evaluated every iteration, and `continue` re-tests it rather than jumping to
the top.

A branch whose type diverges (`return`, or a `loop` with no reachable `break`)
unifies with any other branch's type, as Rust's `!` does, so
`if c { 1 } else { return 2 }` is accepted. The trailing `;` after
`return`/`break`/`continue` may be omitted when it is the last thing in a block,
since those diverge and no expression can follow them. An assignment and a
compound assignment are `()`-valued expressions, so their trailing `;` may be
omitted the same way: `{ x = 1 }` and `{ x += 1 }` are accepted when the
assignment is the last thing in a block. A bare `;` is an empty statement and
may appear between statements or after the last one, as in Rust.
A whole block that
cannot complete is `!` too, so it fits any expected type: `fn f(e: E) -> i64 {
match e { E::A => { return 1; } E::B => { return 2; } } }` (every arm returns),
`let x: i64 = if c { return 1; } else { return 2; };`, and `fn g() -> i64 {
loop {} }` are all accepted, matching rustc. An *unannotated* binding whose
initializer diverges keeps type `!`, though, and rmc has no usage-based
inference to refine it, so using it as a place is rejected: both
`let x = { return 3; }; x + 1` and `let S { a } = { return 3; };` are errors
where rustc accepts them (it never needs the value). Add an annotation
(`let x: i64 = { return 3; };`) to accept them.

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
let` (`'a: while let PAT = EXPR { .. }`) attaches to the `loop` the statement
desugars to, so `break 'a` / `continue 'a` in the body target it just as they
would a labelled `while`.

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
- Comparison: `== != < <= > >=`, producing `bool`. Comparison is
  non-associative, as in Rust: `a < b < c` is a hard error, not `(a < b) < c`;
  parenthesise the inner test (`(a < b) == c`) to chain it.
- Boolean: `&& || !`, short-circuiting. `!` is logical not on a `bool` and
  bitwise not on an `i64`, as in Rust; it yields its operand's type.
- Bitwise: `& | ^` on `i64`, and on `bool` where they are the non-short-circuiting
  logical operators (`a & b` is `a && b` without the short circuit), as in Rust;
  `a & b` and `a | b` yield `bool` when both operands are `bool`. Also `<<` and
  `>>`. `>>` is arithmetic (a negative
  value stays negative) and a shift count is taken modulo 64, matching Rust.
  Precedence, tightest first: `* / %`, `+ -`, `<< >>`, `&`, `^`, `|`, then the
  comparisons and `&& ||`. So `1 + 2 << 3` is `(1 + 2) << 3`, and
  `if x & 1 == 0` is `(x & 1) == 0`. A cast `e as T` binds looser than the unary
  operators and tighter than `*`, so `*r as i64` is `(*r) as i64` and
  `-x as i64` is `(-x) as i64`, as in Rust. rmc has only `i64`, so a literal
  count is masked to 6 bits; in Rust an unconstrained literal count is `i32` and
  masks to 5, which is why a program that mixes the two types can differ.
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
  copy. Because `r.m()` resolves through the reference, a built-in `i64` method
  on a bare `&i64` receiver (`r.min(0)`) is accepted where rustc binds
  `<&i64>::min` and demands a reference argument; write `(*r).min(0)` or
  `r.min(&0)` to match rustc. rmc is a superset here, never a wrong answer.
  There is no borrow checking: rmc does not track how long a reference
  lives, so it is the programmer's job not to outlive the referent, and `&mut` is
  not enforced to be exclusive.
- Calls: `f(a, b)`.
- Indexing: `arr[i]`.
- Array literals: `[a, b, c]` and `[value; count]`. A list literal takes its
  element type from the values, so `[true, false]` is a `[bool; 2]` and
  `[1, 2, 3]` an `[i64; 3]`; mixing the two is an error. An array element may be
  a scalar (`i64`, `bool`, a unit enum), a struct or another array, so
  `[[i64; 3]; 2]`, `[Point; 4]` and `[[[i64; 2]; 2]; 2]` are all valid; a payload
  enum and a reference are rejected (rustc rejects both as well). Element `i`
  lives at `base + i * sizeof(T)`, so a wide element (a nested array or a struct)
  occupies `size / 8` slots, and a whole element may be read, written or copied
  (`a[i] = b[j]`, `m[i]` as a `[i64; 3]` value).
- Struct literals: `Point { x: 1, y: 2 }`, usable as a `let` initialiser.
- Blocks and `if`/`else` are expressions.
- `match` on integers, characters, enums or strings: `match x { 0 => a, 1 => b,
  _ => c }`, `match c { 'a'..='z' => a, _ => b }`, `match c { Color::Red => a, _
  => b }` or `match s { "on" => a, _ => b }`. An
  arm pattern is an integer literal (optionally negated), an integer *range*
  `lo..hi` / `lo..=hi`, a *character* literal, a character *range*
  `'a'..'z'` / `'a'..='z'`, a *string* literal, an `Enum::Variant`, a struct
  pattern `S { f, .. }` (see below), a *tuple* pattern `(a, b)` (see below),
  an *or-pattern* `a | b | c`, an *at-binding* `n @ PAT` (see below), or `_`. The
  first matching arm
  runs and `_` matches anything; it is an expression, usable as a statement or a
  value, and a comma before `}` may be omitted. An arm body is an expression,
  but a bare assignment (`0 => acc = 1`), a compound assignment
  (`1 => acc += 10`), or a `return`/`break`/`continue` is accepted as well, as
  in Rust; a block-bodied arm (`_ => {}`) also works. If the scrutinee is a
  reference, the pattern reads through it, as Rust's match ergonomics do:
  `match &e { Opt::Some(v) => .. }` dereferences `&E` to the `E` underneath, and
  a struct pattern likewise auto-derefs a `&S` scrutinee. A scalar pattern does
  too: `match &x { 5 => .. }`, `match &x { 1..=10 => .. }` and
  `match &f { true => .. }` read the referent, so an explicit `*` is not needed.
  Only a pattern auto-derefs — a bare comparison such as `r < 6` still needs
  `*r`, exactly as in Rust. The referent is what the pattern is checked against.
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
- Struct patterns: `S { f, g: h, i: _, .. }` matches a `S` scrutinee and binds
  its fields. A field may be shorthand (`f` binds `f`), renamed (`g: h` binds
  `h`), or ignored (`i: _`); a trailing `..` drops the fields not named, and
  `S { .. }` binds nothing. A binding may be made mutable with `mut`, written on
  the binding (`mut f`, `g: mut h`), not on the pattern: `mut` is a per-binding
  flag, as in rustc. A struct pattern is *irrefutable*: it has no runtime
  test and always matches, so it acts as a catch-all arm (like `_`) and needs no
  other arm to make a `match` exhaustive. A field is an ordinary place read, so
  it may auto-deref a `&S` scrutinee. A struct-typed field may itself be
  destructured by a nested struct sub-pattern, `i: I { a }` (to any depth), which
  expands to a chain of field reads; the sub-pattern binds its own fields and may
  use shorthand, renames, `_` and `..` like the top level. An array-typed field
  may likewise be destructured by an array pattern, `a: [x, y]`, whose elements
  may in turn be struct patterns, so the two nesting directions compose. `mut`
  attaches to a
  binding, so `i: mut I { a }` and `a: mut [x, y]` are rejected, as in rustc. An
  enum payload pattern's arguments take the same sub-patterns (see the payload
  enum section). A wrong field name is an error,
  even for `i: _`, and a field bound twice is an error. Because a struct pattern
  is irrefutable, a *binding* struct pattern in an or-pattern is rejected, as is
  any binding alternative (a nested binding counts).
- At-bindings: `name @ subpat` binds the whole value to `name` and also matches
  it against `subpat`, so the arm body (or the rest of the block) sees both the
  whole value and the sub-pattern's bindings. `name @ _` is the plain alias, and
  the two compose: `n @ m @ _` binds the value to both `n` and `m`. A `mut`
  before the name (`mut n @ subpat`) makes the alias mutable. `subpat` may be
  `_`, a binding, a struct, tuple or array pattern, or — in a refutable position
  — a literal, a range, an or-pattern or a variant. An at-binding is accepted
  everywhere a pattern is: a `match` arm (`n @ 1..=5 => ..`), an `if let` /
  `while let`, a struct field (`S { x: n @ _ }`), an array element
  (`[n @ _, m]`), a `let`, a `for` head and a parameter
  (`fn f(n @ _: i64)`). Where the position is irrefutable (`let`, `for`,
  parameters, and the sub-pattern of a struct field), `subpat` must be
  irrefutable too, so `let n @ 1..=5 = e;` and `for n @ 1..=5 in ..` are
  rejected, as rustc rejects a refutable local binding (`E0005`). A binding
  alternative inside an or-pattern is still rejected: `n @ 1 | 2` is `E0408`.
  An at-binding whose sub-pattern is a `..` rest (`first @ ..`, a sub-slice
  binding) is not supported, as rmc has no slice type.
- `let` destructuring: `let S { f, g: h, i: _, .. } = e;` uses the same struct
  pattern grammar and binds the named fields as fresh locals in the enclosing
  block, so they stay visible for the rest of it. The pattern is irrefutable by
  construction, so there is no test; a type annotation is allowed after the
  pattern (`let S { x, y }: S = e;`) and is checked against the scrutinee. The
  scrutinee is evaluated exactly once. `mut` goes on each binding, so
  `let mut S { .. } = e;` is rejected (`mut` must be attached to each individual
  binding), while `let S { mut x, y: mut z } = e;` is accepted. As in `match`, a
  field read may auto-deref a `&S` scrutinee, and a struct-typed field may be
  destructured by a nested sub-pattern (`let S { a: I { x } } = e;`), which
  expands to a chain of field reads; an array-typed field may be destructured by
  an array pattern (`let S { a: [x, y] } = e;`), which expands to field reads
  followed by index reads. An at-binding is accepted here too
  (`let n @ _ = e;`, `let t @ (a, b) = e;`, `let S { x: n @ _ } = e;`), binding
  the whole value as well as the sub-pattern's names; `subpat` must be
  irrefutable, so `let n @ 1..=5 = e;` is rejected.
- Array patterns: `[a, b, .., c]` matches an array and binds its elements. Each
  element is a sub-pattern: a binding (`a`), an ignored element (`_`), or a
  nested array or struct pattern (`[[p, q], s]`, `[P { x }, y]`). A single `..`
  absorbs any number of middle elements, so `[first, ..]`, `[.., last]` and
  `[x, .., z]` all work; a second `..` is rejected. An element may be made
  mutable with `mut` on the binding (`[mut a, b]`), not on the pattern as a
  whole (`let mut [a, b] = e;` is rejected, as in rustc). An array pattern is
  *irrefutable* when every element pattern is, so it has no runtime test and
  acts as a catch-all: it works in `let` (`let [a, b] = e;`), `for`
  (`for [x, y] in pairs`), `match` (`match m { [a, b] => .. }`), `if let` and
  `while let`, and as an array-typed struct field's sub-pattern
  (`let S { a: [x, y], b } = s;`). It expands to one index read per element, so
  a fixed element `j` reads `subj[j]` and a trailing element reads
  `subj[subj.len() - k]` relative to the array's length. An element pattern must
  be irrefutable: a literal element (`[1, y]`) is rejected rather than
  mis-bound. An array pattern that binds
  nothing (`[_, _]`, `[..]`) is accepted and still evaluates the scrutinee once.
  Without a `..` rest, the pattern must name exactly the array's length, as
  rustc requires: `let [x] = [1, 2];` is rejected (E0527), not silently bound to
  the first element.
- Tuple patterns: `(a, b)` matches a tuple and binds its elements, in a `let`,
  `for` head, parameter, `match` arm, `if let` or `while let`. An element is a
  binding, `_`, or a nested struct/array/tuple pattern, exactly as an array
  element is; `mut` goes on the binding. A tuple pattern expands to one field
  read per element (element `k` is field `"k"` of the anonymous struct the tuple
  lowers to). In a `match` arm or an `if let` / `while let` a tuple pattern may
  be *refutable*: an element may be a scalar test (an integer, `true`/`false`, a
  string, a range, or an or-pattern of those), and the arm runs only when every
  element matches; a binding element is read through the test. A refutable
  element may not itself bind or be a variant (`(E::V(x), y)`), since rmc
  generates no test that carries a payload. In a `let`, `for` head or parameter
  the pattern is irrefutable, so a literal element is rejected, as rustc
  rejects it. The pattern must name exactly the tuple's arity (E0308):
  `let (x,) = (1, 2);` is rejected, not bound to the first field.
- `for` destructuring: `for S { f, g: h, i: _, .. } in arr { .. }` or
  `for [x, y] in arr { .. }` iterates the array as above but destructures each
  element with the same struct- or array-pattern grammar, binding the named
  fields or elements fresh each iteration. The array head is evaluated once,
  outside the loop. `break`/`continue` and a label behave as for any other
  `for`. The pattern must be irrefutable, so only a binding, `_`, a struct
  pattern or an array pattern is allowed; `mut` goes on each binding, as in
  `let` destructuring.
- Parameter patterns: `fn f(S { f, g: h, i: _, .. }: S)` or
  `fn f([a, b]: [i64; 2])` writes a parameter as a pattern. The parameter itself
  is an ordinary argument of the annotation's type; the pattern binds its names
  at the top of the body, exactly as the matching `let` destructuring would, so
  the same field- and element-read expansion applies and the names read through
  the parameter. The pattern must be irrefutable, so only a binding, a struct
  pattern or an array pattern is allowed (a literal or variant is rejected, as
  rustc rejects it); `mut` goes on each binding, so `mut S { x }: S` is rejected.
  A name a pattern binds may not collide with another parameter (rustc's
  `E0415`).
- `if let PAT = EXPR { .. } else { .. }`, sugar for
  `match EXPR { PAT => then, _ => else_or_unit }`. It reuses the `match` pattern
  grammar (including or-patterns, ranges, struct patterns and array patterns,
  `if let 1..=5 = x`), but unlike
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
| `print_char(c)` | `(char) -> ()` |
| `putchar(c)`    | `(i64) -> ()`  |
| `alloc(n)`      | `(i64) -> i64` |
| `free(p)`       | `(i64) -> ()`  |
| `arg_i64(i)`    | `(i64) -> i64` |
| `exit(code)`    | `(i64) -> ()`  |

`alloc` returns a pointer to `n` bytes of writable memory, or 0 on failure; it
is a first-fit allocator over an mmap'd arena, and `free` returns a block for
reuse and coalesces neighbours. Memory is not zeroed.

`exit(code)` terminates the process with `code` via the raw `exit` syscall. It
is the target of `std::process::exit`, a failed `assert!`, and `panic!`.

### Macros

`print!` and `println!` format a literal string and print it. The format string
must be a literal; each `{}` is filled by the next argument in order, and is
rendered by the argument's type (`i64` in decimal, `bool` as `true`/`false`,
`char` as the character itself, `str` as its text). `{{` and `}}` are literal
braces, and the usual escapes
(`\n`, `\t`, `\"`, ...) are resolved by the lexer. `println!` appends a newline;
`print!` adds none.

`assert!(cond)`, `assert!(cond, msg)`, `assert_eq!(a, b)` and `assert_ne!(a, b)`
evaluate their condition and, when it fails, exit with status 101 (rustc's panic
code). The optional message is not printed. `panic!`, `unreachable!`, `todo!`
and `unimplemented!` always exit 101; their arguments are ignored.

### Vec shims

rmc has no `Vec`. A few `Vec` shapes appear in real Rust, so the parser folds
each to an `i64` address into the same heap `alloc` uses, plus a runtime helper.
The value of a supported `Vec` binding is that address; it is not a container
with a length or capacity field, so only the methods below are understood.

| source                              | lowered to                                    |
| ----------------------------------- | --------------------------------------------- |
| `let v: Vec<i64> = Vec::with_capacity(k);` | `alloc(k * 8)`; `v.capacity()` is `k`  |
| `let v: Vec<u8> = vec![e; n];`      | `alloc(n)` filled by `__rmc_fill_bytes` (one byte per element) |
| `v.len()` (constant `n`)            | `n`                                           |
| `v.as_mut_ptr()` / `v.as_ptr()`     | the address; a following `.add(off)` is `p + off * elem` |
| `let a: Vec<i64> = std::env::args().skip(k).map(..).collect();` | no allocation at all (see below) |

`vec![e; n]` takes a runtime `n`; `alloc` and the fill helper both take it as a
value, and the allocation is `n * elem_width` bytes (`elem_width` is 1 for a
`u8`/`i8` element and 8 otherwise, so a `Vec<u8>` allocates `n` bytes and a
`Vec<i64>` `n` words). A `vec![a, b, c]` list form is rejected. `add(off)` scales `off` by the
element width taken from the `Vec<T>` annotation, so a `Vec<i64>` pointer steps
a whole word while a `Vec<u8>` stays byte-addressed. An args-vector is never
materialised: `for v in &args` desugars to `for i in k..__rmc_nargs()` and
`args[i]` to `__rmc_arg_or(i + k, 0)`, so `*v` in the loop body is the `i64`
argument itself. Each of these constructs is only recognised directly in a
`let` initializer; used elsewhere it is a compile error naming the construct.

### std paths

Two `std` items are provided so ordinary Rust can be compiled unchanged:

| path                        | meaning                      |
| --------------------------- | ---------------------------- |
| `std::process::exit(code)`  | the `exit(code)` builtin     |
| `std::hint::black_box(x)`   | the identity (`x`)           |

`use` statements are parsed and discarded at the top level and inside a block:
rmc resolves every name globally, so an import has nothing to do. A `use` of a
`std` subtree is remembered, though, so after `use std::process;` a later
`process::exit(0)` resolves to the same item as the full `std::process::exit`.
Any other `std::` path is a compile error.

A `pub` visibility qualifier — on an item, a struct field, an `impl` method or a
nested `fn` — is parsed and dropped, along with any restriction
(`pub(crate)`, `pub(super)`, `pub(in ..)`). rmc resolves every name in one flat
namespace, so visibility carries no meaning; accepting it lets an ordinary Rust
source file parse unchanged. An inline `mod NAME { .. }` is likewise dropped:
its body is not visible, but a program that declares one and uses only the flat
items still compiles. A `mod NAME;` that would load another file is a compile
error, since rmc compiles a single file and cannot resolve it.

## Not supported

Traits, generics, borrowing, lifetimes, modules, closures, tuple structs
(`struct S(i64, i64)`), raw string literals (`r".."`), `f32`/`f64`,
`u*`/`i*` types other than `i64`, and the standard library beyond the two `std`
paths above and the `Vec` shims. A `Vec` is not a real container: only the
handful of shapes in "Vec shims" are recognised, and any other `Vec` method or
use is a compile error. A `char` is a scalar, but its full `char` API
(`char::from_u32`, `is_alphabetic`, `to_digit`, ...) is not provided, and
`char` may not be cast from an integer (`i64 as char` is rejected, as in
rustc). References exist (`&T` / `&mut T`, `&e`, `*e`, `*e = v`),
but without borrow checking: rmc does not prove that a reference outlives its
referent, and `&mut` carries no exclusivity guarantee. An aggregate is still
passed by value unless a reference is used. A `str` and a `&str` are the same
one-word value (the address of the `{len, bytes}` record), so the two coerce
freely; a `&'static str` field or `&str` parameter may be written `str`.
