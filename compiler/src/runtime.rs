//! Minimal freestanding runtime, written directly in x86-64 assembly.
//!
//! There is no libc: process start-up, integer/string printing, memory
//! allocation and exit all go through raw Linux syscalls. This keeps the
//! produced binaries tiny and independent of any C runtime.
//!
//! The runtime is stored as a list of [`Piece`]s rather than one blob so that
//! [`for_program`] can link only the parts a program actually reaches. A
//! program that never allocates should not carry an allocator, and one that
//! never indexes an array should not carry the bounds-check trap; on a binary
//! this small that dead weight is a large fraction of the artifact.

use std::collections::HashSet;
use std::sync::OnceLock;

/// One selectable piece of the runtime: its assembly text and the global
/// symbols it defines. A piece is kept when the program — or another kept
/// piece — names one of its symbols.
struct Piece {
    text: &'static str,
    defines: &'static [&'static str],
}

/// The runtime, in emission order. Every piece starts by naming its own
/// section, so any subset concatenates into a valid translation unit.
static PIECES: &[Piece] = &[
    Piece {
        text: r#"
.data
.align 8
__rmc_argc: .quad 0
__rmc_argv: .quad 0
"#,
        defines: &["__rmc_argc", "__rmc_argv"],
    },
    Piece {
        text: r#"
.data
.align 8
# First free block, or 0. The free list is address-ordered so neighbouring
# blocks can be coalesced on free.
__rmc_freelist: .quad 0
# Bump pointer and end of the current mmap'd chunk. Fresh blocks are carved
# from here; freed blocks go back on the free list for reuse.
__rmc_arena: .quad 0
__rmc_arena_end: .quad 0
"#,
        defines: &["__rmc_freelist", "__rmc_arena", "__rmc_arena_end"],
    },
    Piece {
        text: r#"
.text
.globl _start
.type _start, @function
_start:
    xor %rbp, %rbp
    mov (%rsp), %rax
    mov %rax, __rmc_argc(%rip)
    lea 8(%rsp), %rax
    mov %rax, __rmc_argv(%rip)
    and $-16, %rsp
    call main
    mov %rax, %rdi
    mov $60, %rax
    syscall
.size _start, .-_start
"#,
        defines: &["_start"],
    },
    Piece {
        text: r#"
# long arg_i64(long idx)  -- parse argv[idx] as a signed decimal integer.
# Returns 0 when the index is out of range. Used by benchmarks so the input is
# opaque to the compiler and cannot be constant-folded away.
.globl arg_i64
.type arg_i64, @function
arg_i64:
    push %rbp
    mov %rsp, %rbp
    mov __rmc_argc(%rip), %rcx
    # A negative index passes `cmp/jge` (signed: -1 < argc) and would read
    # before argv, so reject it explicitly.
    test %rdi, %rdi
    js .Larg_zero
    cmp %rcx, %rdi
    jge .Larg_zero
    mov __rmc_argv(%rip), %rdx
    mov (%rdx,%rdi,8), %rsi
    xor %rax, %rax
    xor %r8, %r8
    movzbl (%rsi), %ecx
    cmp $45, %cl
    jne .Larg_loop
    mov $1, %r8
    inc %rsi
.Larg_loop:
    movzbl (%rsi), %ecx
    cmp $48, %cl
    jl .Larg_done
    cmp $57, %cl
    jg .Larg_done
    imul $10, %rax, %rax
    sub $48, %rcx
    add %rcx, %rax
    inc %rsi
    jmp .Larg_loop
.Larg_done:
    test %r8, %r8
    jz .Larg_ret
    neg %rax
.Larg_ret:
    leave
    ret
.Larg_zero:
    xor %eax, %eax
    leave
    ret
.size arg_i64, .-arg_i64
"#,
        defines: &["arg_i64"],
    },
    Piece {
        text: r#"
# long __rmc_arg_or(long idx, long default)  -- arg_i64(idx) when idx is a
# valid index, else `default`. Lowers the `std::env::args().nth(i)...unwrap_or(d)`
# idiom (and its `.and_then(..).ok()` spelling) without an Option type.
.globl __rmc_arg_or
.type __rmc_arg_or, @function
__rmc_arg_or:
    push %rbp
    mov %rsp, %rbp
    test %rdi, %rdi
    js .Lao_def
    mov __rmc_argc(%rip), %rcx
    cmp %rcx, %rdi
    jge .Lao_def
    call arg_i64
    leave
    ret
.Lao_def:
    mov %rsi, %rax
    leave
    ret
.size __rmc_arg_or, .-__rmc_arg_or
"#,
        defines: &["__rmc_arg_or"],
    },
    Piece {
        text: r#"
# long __rmc_nargs(void)  -- the argument count. The `for v in &args` desugar
# bounds its loop by this, so an args-vector is never materialised.
.globl __rmc_nargs
.type __rmc_nargs, @function
__rmc_nargs:
    mov __rmc_argc(%rip), %rax
    ret
.size __rmc_nargs, .-__rmc_nargs
"#,
        defines: &["__rmc_nargs"],
    },
    Piece {
        text: r#"
# long __rmc_fill(long addr, long count, long elem)  -- store `elem` at each of
# `count` words from `addr`; returns `addr`. Backs the `vec![e; n]` desugar.
.globl __rmc_fill
.type __rmc_fill, @function
__rmc_fill:
    mov %rdi, %r8
    test %rsi, %rsi
    jle .Lfill_done
    mov %rdx, %rax           # the word to store
    mov %rsi, %rcx           # count
    rep stosq
.Lfill_done:
    mov %r8, %rax            # return the original address
    ret
.size __rmc_fill, .-__rmc_fill
"#,
        defines: &["__rmc_fill"],
    },
    Piece {
        text: r#"
# long __rmc_fill_bytes(long addr, long count, long elem)  -- store the low byte
# of `elem` at each of `count` bytes from `addr`; returns `addr`. Backs the
# `vec![0u8; n]` desugar, whose elements are one byte wide. A `rep stosb` fill
# is far faster than a scalar byte loop for the arena sizes `vec![..]` is used
# with.
.globl __rmc_fill_bytes
.type __rmc_fill_bytes, @function
__rmc_fill_bytes:
    mov %rdi, %r8
    test %rsi, %rsi
    jle .Lfillb_done
    mov %rdx, %rax           # `stosb` stores %al
    mov %rsi, %rcx           # count
    rep stosb
.Lfillb_done:
    mov %r8, %rax            # return the original address
    ret
.size __rmc_fill_bytes, .-__rmc_fill_bytes
"#,
        defines: &["__rmc_fill_bytes"],
    },
    Piece {
        text: r#"
# long __rmc_min(long a, long b)  -- the smaller of two signed words. Backs the
# `skip`/`take` range adapters' count, where it caps the number of iterations at
# the range length. Signed compare, since a range length is never negative but
# the operands are `i64`.
.globl __rmc_min
.type __rmc_min, @function
__rmc_min:
    mov %rdi, %rax
    cmp %rsi, %rax
    jle .Lmin_done
    mov %rsi, %rax
.Lmin_done:
    ret
.size __rmc_min, .-__rmc_min
"#,
        defines: &["__rmc_min"],
    },
    Piece {
        text: r#"
# void exit(long code)
.globl exit
.type exit, @function
exit:
    mov $60, %rax
    syscall
.size exit, .-exit
"#,
        defines: &["exit"],
    },
    Piece {
        text: r#"
# void print_i64(long n)  -- decimal value followed by a newline
.globl print_i64
.type print_i64, @function
print_i64:
    push %rbp
    mov %rsp, %rbp
    sub $64, %rsp
    lea -1(%rbp), %rsi
    movb $10, (%rsi)
    mov $1, %rcx
    mov %rdi, %rax
    xor %r8, %r8
    test %rax, %rax
    jns .Lpi_pos
    mov $1, %r8
    neg %rax
.Lpi_pos:
    mov $10, %r9
.Lpi_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %rsi
    mov %dl, (%rsi)
    inc %rcx
    test %rax, %rax
    jnz .Lpi_loop
    test %r8, %r8
    jz .Lpi_write
    dec %rsi
    movb $45, (%rsi)
    inc %rcx
.Lpi_write:
    mov %rcx, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size print_i64, .-print_i64
"#,
        defines: &["print_i64"],
    },
    Piece {
        text: r#"
# void print_str(long ptr)  -- ptr points at a {len: i64, bytes...} record
.globl print_str
.type print_str, @function
print_str:
    push %rbp
    mov %rsp, %rbp
    mov (%rdi), %rdx
    lea 8(%rdi), %rsi
    mov $1, %rax
    mov $1, %rdi
    syscall
    mov $1, %rax
    mov $1, %rdi
    mov $10, %rsi
    push $10
    mov %rsp, %rsi
    mov $1, %rdx
    syscall
    pop %rsi
    leave
    ret
.size print_str, .-print_str
"#,
        defines: &["print_str"],
    },
    Piece {
        text: r#"
# void __rmc_write_i64(long n)  -- decimal value, no trailing newline
.globl __rmc_write_i64
.type __rmc_write_i64, @function
__rmc_write_i64:
    push %rbp
    mov %rsp, %rbp
    sub $64, %rsp
    lea -1(%rbp), %rsi
    mov $0, %rcx
    mov %rdi, %rax
    xor %r8, %r8
    test %rax, %rax
    jns .Lwi_pos
    mov $1, %r8
    neg %rax
.Lwi_pos:
    mov $10, %r9
.Lwi_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %rsi
    mov %dl, (%rsi)
    inc %rcx
    test %rax, %rax
    jnz .Lwi_loop
    test %r8, %r8
    jz .Lwi_write
    dec %rsi
    movb $45, (%rsi)
    inc %rcx
.Lwi_write:
    mov %rcx, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size __rmc_write_i64, .-__rmc_write_i64
"#,
        defines: &["__rmc_write_i64"],
    },
    Piece {
        text: r#"
# void __rmc_write_u64(unsigned long n)  -- unsigned decimal, no trailing newline
.globl __rmc_write_u64
.type __rmc_write_u64, @function
__rmc_write_u64:
    push %rbp
    mov %rsp, %rbp
    sub $64, %rsp
    lea -1(%rbp), %rsi
    mov $0, %rcx
    mov %rdi, %rax
    mov $10, %r9
.Lwu_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %rsi
    mov %dl, (%rsi)
    inc %rcx
    test %rax, %rax
    jnz .Lwu_loop
    mov %rcx, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size __rmc_write_u64, .-__rmc_write_u64
"#,
        defines: &["__rmc_write_u64"],
    },
    Piece {
        text: r#"
# void __rmc_write_bool(bool b)  -- `true` or `false`, no trailing newline
.globl __rmc_write_bool
.type __rmc_write_bool, @function
__rmc_write_bool:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    test %rdi, %rdi
    jz .Lwb_false
    movb $0x74, -8(%rbp)
    movb $0x72, -7(%rbp)
    movb $0x75, -6(%rbp)
    movb $0x65, -5(%rbp)
    lea -8(%rbp), %rsi
    mov $4, %rdx
    jmp .Lwb_write
.Lwb_false:
    movb $0x66, -8(%rbp)
    movb $0x61, -7(%rbp)
    movb $0x6c, -6(%rbp)
    movb $0x73, -5(%rbp)
    movb $0x65, -4(%rbp)
    lea -8(%rbp), %rsi
    mov $5, %rdx
.Lwb_write:
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size __rmc_write_bool, .-__rmc_write_bool
"#,
        defines: &["__rmc_write_bool"],
    },
    Piece {
        text: r#"
# void __rmc_write_str(long ptr)  -- ptr points at a {len: i64, bytes...} record,
# written with no trailing newline (unlike print_str)
.globl __rmc_write_str
.type __rmc_write_str, @function
__rmc_write_str:
    push %rbp
    mov %rsp, %rbp
    mov (%rdi), %rdx
    lea 8(%rdi), %rsi
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size __rmc_write_str, .-__rmc_write_str
"#,
        defines: &["__rmc_write_str"],
    },
    Piece {
        text: r#"
# void __rmc_write_char(long c)  -- c is a Unicode scalar value, written as its
# UTF-8 bytes with no trailing newline (like `{}` for a `char`).
.globl __rmc_write_char
.type __rmc_write_char, @function
__rmc_write_char:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    mov %edi, %eax
    cmp $0x80, %eax
    jae .Lwc_two
    mov %al, -8(%rbp)
    mov $1, %edx
    jmp .Lwc_write
.Lwc_two:
    cmp $0x800, %eax
    jae .Lwc_three
    mov %eax, %ecx
    shr $6, %ecx
    or $0xC0, %ecx
    mov %cl, -8(%rbp)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -7(%rbp)
    mov $2, %edx
    jmp .Lwc_write
.Lwc_three:
    cmp $0x10000, %eax
    jae .Lwc_four
    mov %eax, %ecx
    shr $12, %ecx
    or $0xE0, %ecx
    mov %cl, -8(%rbp)
    mov %eax, %ecx
    shr $6, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -7(%rbp)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -6(%rbp)
    mov $3, %edx
    jmp .Lwc_write
.Lwc_four:
    mov %eax, %ecx
    shr $18, %ecx
    or $0xF0, %ecx
    mov %cl, -8(%rbp)
    mov %eax, %ecx
    shr $12, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -7(%rbp)
    mov %eax, %ecx
    shr $6, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -6(%rbp)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, -5(%rbp)
    mov $4, %edx
.Lwc_write:
    lea -8(%rbp), %rsi
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size __rmc_write_char, .-__rmc_write_char
"#,
        defines: &["__rmc_write_char"],
    },
    Piece {
        text: r#"
# void print_char(char c)  -- the character written as UTF-8 bytes followed by
# a newline, the char analogue of print_i64/print_bool.
.globl print_char
.type print_char, @function
print_char:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    call __rmc_write_char
    movb $10, -8(%rbp)
    lea -8(%rbp), %rsi
    mov $1, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size print_char, .-print_char
"#,
        defines: &["print_char"],
    },
    Piece {
        text: r#"
# bool __rmc_str_lt(long a, long b)  -- a and b point at {len: i64, bytes...}
# records. Returns 1 when the byte string a is lexicographically less than b,
# 0 otherwise. A `str` value is the address of its record, so a string
# comparison must look at the bytes rather than the pointers; sema lowers
# `==`, `!=`, `<`, `<=`, `>` and `>=` on two strings to calls to this helper.
.globl __rmc_str_lt
.type __rmc_str_lt, @function
__rmc_str_lt:
    mov (%rdi), %r8          # len(a)
    mov (%rsi), %r9          # len(b)
    lea 8(%rdi), %rdi        # first byte of a
    lea 8(%rsi), %rsi        # first byte of b
    mov %r8, %rdx
    cmp %r9, %rdx
    jbe .Lsl_min
    mov %r9, %rdx            # rdx = min(len(a), len(b))
.Lsl_min:
    xor %rcx, %rcx           # i = 0
.Lsl_loop:
    cmp %rdx, %rcx
    jae .Lsl_prefix
    movzbl (%rdi,%rcx), %eax
    movzbl (%rsi,%rcx), %r10d
    cmp %r10d, %eax
    jb .Lsl_true
    ja .Lsl_false
    inc %rcx
    jmp .Lsl_loop
.Lsl_prefix:
    # Equal up to the shorter length: a < b iff a is shorter.
    xor %eax, %eax
    cmp %r9, %r8
    setb %al
    ret
.Lsl_true:
    mov $1, %eax
    ret
.Lsl_false:
    xor %eax, %eax
    ret
.size __rmc_str_lt, .-__rmc_str_lt
"#,
        defines: &["__rmc_str_lt"],
    },
    Piece {
        text: r#"
# long __rmc_str_len(long s)  -- the byte length of the string record at `s`,
# which is `{len: i64, bytes...}`. A `str` value is the address of its record,
# so `s.len()` is the record's first word; sema lowers it to a call here.
.globl __rmc_str_len
.type __rmc_str_len, @function
__rmc_str_len:
    mov (%rdi), %rax         # len(s)
    ret
.size __rmc_str_len, .-__rmc_str_len
"#,
        defines: &["__rmc_str_len"],
    },
    Piece {
        text: r#"
# long __rmc_str_starts(long s, long pre)  -- 1 when the bytes of `pre` are a
# prefix of `s`, 0 otherwise. Both are `{len: i64, bytes...}` records, so a
# prefix is `pre.len <= s.len` and the first `pre.len` bytes equal.
.globl __rmc_str_starts
.type __rmc_str_starts, @function
__rmc_str_starts:
    mov (%rdi), %r8          # len(s)
    mov (%rsi), %r9          # len(pre)
    cmp %r9, %r8
    jb .Lss_no               # s is shorter than the prefix
    lea 8(%rdi), %rdi
    lea 8(%rsi), %rsi
    xor %rcx, %rcx
.Lss_loop:
    cmp %r9, %rcx
    jae .Lss_yes
    movzbl (%rdi,%rcx), %eax
    movzbl (%rsi,%rcx), %edx
    cmp %edx, %eax
    jne .Lss_no
    inc %rcx
    jmp .Lss_loop
.Lss_yes:
    mov $1, %eax
    ret
.Lss_no:
    xor %eax, %eax
    ret
.size __rmc_str_starts, .-__rmc_str_starts
"#,
        defines: &["__rmc_str_starts"],
    },
    Piece {
        text: r#"
# long __rmc_str_ends(long s, long suf)  -- 1 when the bytes of `suf` are a
# suffix of `s`, 0 otherwise. Both are `{len: i64, bytes...}` records, so a
# suffix is `suf.len <= s.len` and the last `suf.len` bytes equal.
.globl __rmc_str_ends
.type __rmc_str_ends, @function
__rmc_str_ends:
    mov (%rdi), %r8          # len(s)
    mov (%rsi), %r9          # len(suf)
    cmp %r9, %r8
    jb .Lse_no               # s is shorter than the suffix
    lea 8(%rdi), %rdi
    lea 8(%rsi), %rsi
    mov %r8, %rdx
    sub %r9, %rdx            # rdx = len(s) - len(suf): the suffix's start
    xor %rcx, %rcx
.Lse_loop:
    cmp %r9, %rcx
    jae .Lse_yes
    movzbl (%rdi,%rdx), %eax
    movzbl (%rsi,%rcx), %r10d
    cmp %r10d, %eax
    jne .Lse_no
    inc %rcx
    inc %rdx
    jmp .Lse_loop
.Lse_yes:
    mov $1, %eax
    ret
.Lse_no:
    xor %eax, %eax
    ret
.size __rmc_str_ends, .-__rmc_str_ends
"#,
        defines: &["__rmc_str_ends"],
    },
    Piece {
        text: r#"
# long __rmc_str_find(long s, long needle)  -- the byte index of the first
# occurrence of `needle`'s bytes inside `s`, or -1 when absent. Both are
# `{len: i64, bytes...}` records. An empty needle matches at index 0.
.globl __rmc_str_find
.type __rmc_str_find, @function
__rmc_str_find:
    mov (%rdi), %r8          # len(s)
    mov (%rsi), %r9          # len(needle)
    test %r9, %r9
    jz .Lsf_zero             # an empty needle is at 0
    cmp %r9, %r8
    jb .Lsf_none             # the needle is longer than the haystack
    lea 8(%rdi), %rdi
    lea 8(%rsi), %rsi
    sub %r9, %r8
    inc %r8                  # last start index: len(s) - len(needle)
    xor %rcx, %rcx           # i = 0
.Lsf_outer:
    cmp %r8, %rcx
    jae .Lsf_none
    xor %rdx, %rdx           # j = 0
.Lsf_inner:
    cmp %r9, %rdx
    jae .Lsf_found
    lea (%rdi,%rcx), %r10
    movzbl (%r10,%rdx), %eax
    movzbl (%rsi,%rdx), %r10d
    cmp %r10d, %eax
    jne .Lsf_next
    inc %rdx
    jmp .Lsf_inner
.Lsf_next:
    inc %rcx
    jmp .Lsf_outer
.Lsf_found:
    mov %rcx, %rax
    ret
.Lsf_zero:
    xor %eax, %eax
    ret
.Lsf_none:
    mov $-1, %rax
    ret
.size __rmc_str_find, .-__rmc_str_find
"#,
        defines: &["__rmc_str_find"],
    },
    Piece {
        text: r#"
# void print_bool(bool b)  -- prints 0 or 1 followed by a newline
.globl print_bool
.type print_bool, @function
print_bool:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    test %rdi, %rdi
    setne %al
    add $48, %al
    movb %al, -8(%rbp)
    movb $10, -7(%rbp)
    lea -8(%rbp), %rsi
    mov $2, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size print_bool, .-print_bool
"#,
        defines: &["print_bool"],
    },
    Piece {
        text: r#"
# void putchar(long c)
.globl putchar
.type putchar, @function
putchar:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    mov %dil, -8(%rbp)
    lea -8(%rbp), %rsi
    mov $1, %rdx
    mov $1, %rax
    mov $1, %rdi
    syscall
    leave
    ret
.size putchar, .-putchar
"#,
        defines: &["putchar"],
    },
    Piece {
        text: r#"
# long alloc(long n)  -- first-fit block from the free list, else carve from the
# arena, else mmap a fresh chunk. Returns a pointer with room for `n` bytes.
# Layout of a block: [size: 8 bytes][payload ...]; the pointer handed out points
# at the payload, and the header sits 8 bytes before it.
.globl alloc
.type alloc, @function
alloc:
    # Leaf function: the fast paths touch only caller-saved registers, so there
    # is no frame pointer and no callee-saved save/restore. The one slow path
    # (mmap) preserves the request size on the stack across the syscall, which
    # clobbers %rcx and %r11.
    add $7, %rdi
    and $-8, %rdi
    mov %rdi, %rsi           # requested payload size
    test %rsi, %rsi
    jnz .Lalloc_sz_ok
    mov $8, %rsi             # never hand out a zero-size block
.Lalloc_sz_ok:
    mov %rsi, %r9            # total size including the 8-byte header
    add $8, %r9

    # Walk the free list for the first block large enough (first fit).
    lea __rmc_freelist(%rip), %r10   # pointer to the link we are examining
    mov (%r10), %rax
.Lalloc_scan:
    test %rax, %rax
    jz .Lalloc_from_arena
    mov (%rax), %rcx         # block total size
    cmp %r9, %rcx
    jae .Lalloc_found
    lea 8(%rax), %r10        # next link lives at block+8
    mov (%r10), %rax
    jmp .Lalloc_scan
.Lalloc_found:
    mov 8(%rax), %rdx        # successor link
    mov %rdx, (%r10)
    lea 8(%rax), %rax        # payload
    ret

.Lalloc_from_arena:
    mov __rmc_arena(%rip), %rax
    mov __rmc_arena_end(%rip), %rdx
    mov %rdx, %rcx           # bytes left in the current chunk
    sub %rax, %rcx
    cmp %r9, %rcx
    jae .Lalloc_carve
    # Need a new chunk. Map max(1 MiB, 2 * request), page rounded. The syscall
    # clobbers %rcx/%r11, so the total size rides the stack and the chunk length
    # stays in %rsi (preserved).
    push %r9
    mov $1048576, %rsi
    lea (%r9,%r9), %rcx
    cmp %rsi, %rcx
    jbe .Lalloc_have_size
    mov %rcx, %rsi
.Lalloc_have_size:
    add $4095, %rsi
    and $-4096, %rsi         # chunk length
    xor %rdi, %rdi
    mov $3, %rdx             # PROT_READ|PROT_WRITE
    mov $0x22, %r10          # MAP_PRIVATE|MAP_ANONYMOUS
    mov $-1, %r8
    xor %r9, %r9
    mov $9, %rax             # mmap
    syscall
    pop %r9
    cmp $-4095, %rax
    jae .Lalloc_fail
    mov %rax, __rmc_arena(%rip)
    add %rsi, %rax
    mov %rax, __rmc_arena_end(%rip)
    mov __rmc_arena(%rip), %rax
.Lalloc_carve:
    mov %r9, (%rax)          # header
    lea 8(%rax), %rdx
    add %r9, %rax
    mov %rax, __rmc_arena(%rip)
    mov %rdx, %rax           # payload
    ret
.Lalloc_fail:
    xor %eax, %eax
    ret
.size alloc, .-alloc
"#,
        defines: &["alloc"],
    },
    Piece {
        text: r#"
# void free(long p)  -- return a block to the free list, keeping the list
# address-ordered so adjacent blocks can be coalesced.
.globl free
.type free, @function
free:
    push %rbp
    mov %rsp, %rbp
    push %r12
    push %r13
    push %r14
    test %rdi, %rdi
    jz .Lfree_ret            # free(NULL) is a no-op
    lea -8(%rdi), %r12       # block header
    mov (%r12), %r13         # block size
    # Find the first free block that starts after this one.
    lea __rmc_freelist(%rip), %r14
    mov (%r14), %rax
.Lfree_scan:
    test %rax, %rax
    jz .Lfree_insert
    cmp %r12, %rax
    ja .Lfree_insert         # found the successor
    lea 8(%rax), %r14
    mov (%r14), %rax
    jmp .Lfree_scan
.Lfree_insert:
    # %rax is the successor (may be 0); insert %r12 before it.
    mov %rax, 8(%r12)
    mov %r12, (%r14)
    # Coalesce with the successor if they are adjacent.
    test %rax, %rax
    jz .Lfree_try_prev
    lea (%r12,%r13), %rdx
    cmp %rax, %rdx
    jne .Lfree_try_prev
    mov (%rax), %rcx
    add %rcx, %r13
    mov %r13, (%r12)
    mov 8(%rax), %rcx
    mov %rcx, 8(%r12)
.Lfree_try_prev:
    # %r14 points at the link we just set to %r12. If that link is the list
    # head there is no predecessor; otherwise the predecessor block starts 8
    # bytes before its own `next` field.
    lea __rmc_freelist(%rip), %rdx
    cmp %rdx, %r14
    je .Lfree_ret
    lea -8(%r14), %rdx       # predecessor block header
    mov (%rdx), %rcx         # predecessor size
    lea (%rdx,%rcx), %rsi
    cmp %r12, %rsi           # adjacent?
    jne .Lfree_ret
    add %r13, %rcx           # merge: predecessor absorbs our size
    mov %rcx, (%rdx)
    mov 8(%r12), %rcx        # predecessor now points past us
    mov %rcx, 8(%rdx)
.Lfree_ret:
    pop %r14
    pop %r13
    pop %r12
    leave
    ret
.size free, .-free
"#,
        defines: &["free"],
    },
    Piece {
        text: r#"
.text
# ---- growable `Vec<T>` helpers ----
# A `Vec<T>` value is the address of its element store, exactly as before, so
# `.as_ptr()`/`.add(off)` and element indexing stay plain pointer arithmetic.
# The length and capacity live in a 16-byte header just *before* that address:
# `[data-16]` is the length and `[data-8]` the capacity, both in elements. This
# keeps `Vec` a bare word while letting `push` grow it, and it needs no static
# element type at the use site: the element width travels as an argument, as it
# does for `add`/`free`.
#
# rdi = capacity, rsi = element width  ->  rax = data address
.globl __rmc_vec_new
.type __rmc_vec_new, @function
__rmc_vec_new:
    push %rbx
    push %r12
    push %r13
    sub $8, %rsp              # keep %rsp 16-byte aligned at the `call`
    mov %rdi, %r12            # capacity
    mov %rsi, %r13            # element width
    mov %r12, %rdi
    imul %r13, %rdi           # bytes = capacity * width
    add $16, %rdi             # plus the length/capacity header
    call alloc
    mov %rax, %rbx
    movq $0, (%rbx)           # length = 0
    mov %r12, 8(%rbx)         # capacity
    lea 16(%rbx), %rax        # the value is the element store
    add $8, %rsp
    pop %r13
    pop %r12
    pop %rbx
    ret
.size __rmc_vec_new, .-__rmc_vec_new

# rdi = count, rsi = value, rdx = element width  ->  rax = data address.
# `vec![value; count]`: a fresh vector of `count` copies, length == capacity.
.globl __rmc_vec_repeat
.type __rmc_vec_repeat, @function
__rmc_vec_repeat:
    push %rbx
    push %r12
    push %r13
    push %r14
    push %r15
    sub $8, %rsp
    mov %rdi, %r12            # count
    mov %rsi, %r13            # value
    mov %rdx, %r14            # element width
    mov %r12, %rdi
    imul %r14, %rdi
    add $16, %rdi
    call alloc
    mov %rax, %rbx
    mov %r12, (%rbx)          # length = count
    mov %r12, 8(%rbx)         # capacity = count
    lea 16(%rbx), %r15        # data
    mov %r15, %rdi
    mov %r12, %rsi
    mov %r13, %rdx
    cmp $8, %r14
    je .Lvec_rep_word
    call __rmc_fill_bytes     # a narrow element fills `count` bytes
    jmp .Lvec_rep_done
.Lvec_rep_word:
    call __rmc_fill           # a word element fills `count` words
.Lvec_rep_done:
    mov %r15, %rax
    add $8, %rsp
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbx
    ret
.size __rmc_vec_repeat, .-__rmc_vec_repeat

# rdi = vec, rsi = value, rdx = element width  ->  rax = the (possibly moved)
# vector. Appends one element, doubling the store when it is full; the first
# push of an empty vector grows it to a capacity of one.
.globl __rmc_vec_push
.type __rmc_vec_push, @function
__rmc_vec_push:
    push %rbx
    push %r12
    push %r13
    push %r14
    push %r15
    sub $8, %rsp
    mov %rdi, %rbx            # vec
    mov %rsi, %r13            # value
    mov %rdx, %r14            # element width
    mov -16(%rbx), %r12       # length
    mov -8(%rbx), %r15        # capacity
    cmp %r15, %r12
    jb .Lvec_push_room
    # Full: double the capacity, but never below Rust's minimum non-zero
    # capacity (8 elements for a one-byte element, 4 otherwise), matching
    # `RawVec::grow_amortized`.
    shl $1, %r15
    mov $4, %rax
    cmp $1, %r14
    jne .Lvec_push_min
    mov $8, %rax
.Lvec_push_min:
    cmp %rax, %r15
    jae .Lvec_push_have_cap
    mov %rax, %r15
.Lvec_push_have_cap:
    mov %r15, %rdi
    imul %r14, %rdi
    add $16, %rdi
    call alloc
    mov %rax, %r9             # new base
    lea 16(%r9), %r10         # new data
    mov %rbx, %rsi            # old data
    mov %r12, %rcx
    imul %r14, %rcx           # bytes to copy = length * width
    test %rcx, %rcx
    jz .Lvec_push_installed
.Lvec_push_copy:
    mov (%rsi), %dl
    mov %dl, (%r10)
    inc %rsi
    inc %r10
    dec %rcx
    jnz .Lvec_push_copy
.Lvec_push_installed:
    mov %r12, (%r9)           # length
    mov %r15, 8(%r9)          # capacity
    lea 16(%r9), %rbx         # vec = new data
.Lvec_push_room:
    mov %r12, %rax
    imul %r14, %rax           # offset = length * width
    add %rbx, %rax
    cmp $8, %r14
    je .Lvec_push_word
    mov %r13b, (%rax)         # a narrow element stores its low byte
    jmp .Lvec_push_inc
.Lvec_push_word:
    mov %r13, (%rax)
.Lvec_push_inc:
    inc %r12
    mov %r12, -16(%rbx)       # length += 1
    mov %rbx, %rax
    add $8, %rsp
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbx
    ret
.size __rmc_vec_push, .-__rmc_vec_push

.globl __rmc_vec_len
.type __rmc_vec_len, @function
__rmc_vec_len:
    mov -16(%rdi), %rax
    ret
.size __rmc_vec_len, .-__rmc_vec_len

.globl __rmc_vec_cap
.type __rmc_vec_cap, @function
__rmc_vec_cap:
    mov -8(%rdi), %rax
    ret
.size __rmc_vec_cap, .-__rmc_vec_cap
"#,
        defines: &[
            "__rmc_vec_new",
            "__rmc_vec_repeat",
            "__rmc_vec_push",
            "__rmc_vec_len",
            "__rmc_vec_cap",
        ],
    },
    Piece {
        text: r#"
.text
# void __rmc_overflow(void)  -- a checked +,-,* overflowed; write a fixed
# message and exit with status 101, mirroring a Rust panic.
.globl __rmc_overflow
.type __rmc_overflow, @function
__rmc_overflow:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_overflow_msg(%rip), %rsi
    mov $__rmc_overflow_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_overflow, .-__rmc_overflow

.section .rodata
__rmc_overflow_msg: .ascii "rmc: arithmetic overflow\n"
.set __rmc_overflow_len, . - __rmc_overflow_msg
"#,
        defines: &["__rmc_overflow", "__rmc_overflow_msg", "__rmc_overflow_len"],
    },
    Piece {
        text: r#"
.text
# void __rmc_shift_overflow(void)  -- a checked `<<`/`>>` had a count outside
# 0..=63; write a fixed message and exit with status 101, mirroring a Rust panic.
.globl __rmc_shift_overflow
.type __rmc_shift_overflow, @function
__rmc_shift_overflow:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_shift_overflow_msg(%rip), %rsi
    mov $__rmc_shift_overflow_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_shift_overflow, .-__rmc_shift_overflow

.section .rodata
__rmc_shift_overflow_msg: .ascii "rmc: shift count out of range\n"
.set __rmc_shift_overflow_len, . - __rmc_shift_overflow_msg
"#,
        defines: &[
            "__rmc_shift_overflow",
            "__rmc_shift_overflow_msg",
            "__rmc_shift_overflow_len",
        ],
    },
    Piece {
        text: r#"
.text
# void __rmc_divide_overflow(void)  -- an `i64` division or remainder whose
# result overflows (`i64::MIN / -1`, `i64::MIN % -1`); write a fixed message and
# exit with status 101, mirroring a Rust panic. Rust checks this even without
# `overflow-checks`, because `idiv` traps and cannot produce the wrapping value.
.globl __rmc_divide_overflow
.type __rmc_divide_overflow, @function
__rmc_divide_overflow:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_divide_overflow_msg(%rip), %rsi
    mov $__rmc_divide_overflow_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_divide_overflow, .-__rmc_divide_overflow

.section .rodata
__rmc_divide_overflow_msg: .ascii "rmc: division overflow\n"
.set __rmc_divide_overflow_len, . - __rmc_divide_overflow_msg
"#,
        defines: &[
            "__rmc_divide_overflow",
            "__rmc_divide_overflow_msg",
            "__rmc_divide_overflow_len",
        ],
    },
    Piece {
        text: r#"
.text
# void __rmc_divide_by_zero(void)  -- an `i64` division or remainder by zero;
# write a fixed message and exit with status 101, mirroring a Rust panic.
.globl __rmc_divide_by_zero
.type __rmc_divide_by_zero, @function
__rmc_divide_by_zero:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_divide_by_zero_msg(%rip), %rsi
    mov $__rmc_divide_by_zero_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_divide_by_zero, .-__rmc_divide_by_zero

.section .rodata
__rmc_divide_by_zero_msg: .ascii "rmc: division by zero\n"
.set __rmc_divide_by_zero_len, . - __rmc_divide_by_zero_msg
"#,
        defines: &[
            "__rmc_divide_by_zero",
            "__rmc_divide_by_zero_msg",
            "__rmc_divide_by_zero_len",
        ],
    },
    Piece {
        text: r#"
.text
# void __rmc_bounds(void)  -- a checked array index was out of range; write a
# fixed message and exit with status 101, mirroring a Rust panic.
.globl __rmc_bounds
.type __rmc_bounds, @function
__rmc_bounds:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_bounds_msg(%rip), %rsi
    mov $__rmc_bounds_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_bounds, .-__rmc_bounds

.section .rodata
__rmc_bounds_msg: .ascii "rmc: index out of bounds\n"
.set __rmc_bounds_len, . - __rmc_bounds_msg
"#,
        defines: &["__rmc_bounds", "__rmc_bounds_msg", "__rmc_bounds_len"],
    },
    Piece {
        text: r#"
.text
# void __rmc_clamp(void)  -- `i64::clamp` was called with `min > max`; write a
# fixed message and exit with status 101, mirroring a Rust panic. It never
# returns, but sema types the call `i64` so it can sit in one arm of the
# `if min > max { .. } else { .. }` the desugar builds.
.globl __rmc_clamp
.type __rmc_clamp, @function
__rmc_clamp:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_clamp_msg(%rip), %rsi
    mov $__rmc_clamp_len, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_clamp, .-__rmc_clamp

.section .rodata
__rmc_clamp_msg: .ascii "rmc: clamp min > max\n"
.set __rmc_clamp_len, . - __rmc_clamp_msg
"#,
        defines: &["__rmc_clamp", "__rmc_clamp_msg", "__rmc_clamp_len"],
    },
    Piece {
        text: r#"
.text
# long __rmc_count_ones(long x)  -- the number of set bits in `x`. A plain loop
# over the 64 bits: `x & 1` tests the low bit, then `x` is shifted logically so
# the sign bit is not replicated. The count is exact for every input, including
# `i64::MIN`, so the result does not depend on `--overflow-checks`.
.globl __rmc_count_ones
.type __rmc_count_ones, @function
__rmc_count_ones:
    xor %eax, %eax           # count = 0
    mov $64, %ecx            # 64 bits to examine
.Lco_loop:
    mov %rdi, %rdx
    and $1, %rdx
    add %rdx, %rax
    shr $1, %rdi             # logical shift: bring in a zero, not the sign
    dec %ecx
    jne .Lco_loop
    ret
.size __rmc_count_ones, .-__rmc_count_ones
"#,
        defines: &["__rmc_count_ones"],
    },
    Piece {
        text: r#"
.text
# long __rmc_leading_zeros(long x)  -- the number of high-order zero bits before
# the first set bit; 64 when `x` is zero. `rdx` holds the remaining bits and
# `rcx` counts the shifts, which equals the zeros above the top bit: `shr $63`
# peels the current top bit into `rax`, and each time it is clear the value is
# shifted left to expose the next bit. Only supported instructions are used (no
# `bt`). The count is exact for every input, including `i64::MIN`, so the result
# does not depend on `--overflow-checks`.
.globl __rmc_leading_zeros
.type __rmc_leading_zeros, @function
__rmc_leading_zeros:
    mov %rdi, %rdx           # remaining value
    xor %ecx, %ecx           # shifts = zeros above the top bit
.Llz_loop:
    test %rdx, %rdx
    jz .Llz_done             # no bits left: x was zero
    mov %rdx, %rax
    shr $63, %rax            # top bit
    test %rax, %rax
    jnz .Llz_found
    shl $1, %rdx             # bring the next bit up
    inc %rcx
    jmp .Llz_loop
.Llz_found:
    mov %rcx, %rax           # zeros above bit `rcx`
    ret
.Llz_done:
    mov $64, %rax            # x was zero
    ret
.size __rmc_leading_zeros, .-__rmc_leading_zeros
"#,
        defines: &["__rmc_leading_zeros"],
    },
    Piece {
        text: r#"
.text
# long __rmc_trailing_zeros(long x)  -- the number of low-order zero bits before
# the first set bit; 64 when `x` is zero. `rdx` holds the remaining bits and
# `rcx` counts the zeros dropped so far. `and $1` sets ZF when the low bit is
# clear, so the loop shifts it out and continues; the index of the first set bit
# is the count. Only supported instructions are used (no `bt`).
.globl __rmc_trailing_zeros
.type __rmc_trailing_zeros, @function
__rmc_trailing_zeros:
    mov %rdi, %rdx           # remaining value
    xor %ecx, %ecx           # zeros dropped so far
.Ltz_loop:
    test %rdx, %rdx
    jz .Ltz_done             # no bits left: all zero
    mov %rdx, %rax
    and $1, %rax
    jz .Ltz_skip             # low bit clear: shift it out and continue
    mov %rcx, %rax           # first set bit found
    ret
.Ltz_skip:
    shr $1, %rdx
    inc %rcx
    jmp .Ltz_loop
.Ltz_done:
    mov $64, %rax            # x was zero
    ret
.size __rmc_trailing_zeros, .-__rmc_trailing_zeros
"#,
        defines: &["__rmc_trailing_zeros"],
    },
    Piece {
        text: r#"
.text
# long __rmc_swap_bytes(long x)  -- the byte order reversed. A single `bswap`
# on the 64-bit register is exactly rustc's `i64::swap_bytes`; the in-process
# encoder emits the same 0F C8+rd bytes that `as` assembles here.
.globl __rmc_swap_bytes
.type __rmc_swap_bytes, @function
__rmc_swap_bytes:
    mov %rdi, %rax
    bswap %rax
    ret
.size __rmc_swap_bytes, .-__rmc_swap_bytes
"#,
        defines: &["__rmc_swap_bytes"],
    },
    Piece {
        text: r#"
.text
# long __rmc_reverse_bits(long x)  -- the bit order reversed. A plain loop over
# the 64 bits: each step shifts the result left and ORs in the low bit of the
# remaining value, which is then shifted right. Like the other bit helpers it
# neither negates nor decrements, so the result is exact for every input,
# including `i64::MIN`, and does not depend on `--overflow-checks`.
.globl __rmc_reverse_bits
.type __rmc_reverse_bits, @function
__rmc_reverse_bits:
    xor %eax, %eax           # result = 0
    mov $64, %ecx            # 64 bits to move
.Lrb_loop:
    shl $1, %rax             # result <<= 1, opening the low bit
    mov %rdi, %rdx
    and $1, %rdx             # low bit of the remaining value
    or %rdx, %rax            # drop it into the result
    shr $1, %rdi             # logical shift: bring in a zero, not the sign
    dec %ecx
    jne .Lrb_loop
    ret
.size __rmc_reverse_bits, .-__rmc_reverse_bits
"#,
        defines: &["__rmc_reverse_bits"],
    },
    Piece {
        text: r#"
.text
# long __rmc_rotate_left(long x, long n)  -- the bits rotated left by `n`. The
# count goes in `%cl`, and `rol %cl` masks it to 6 bits (0..63) in hardware,
# exactly Rust's `n % 64`; `n` is never negated, so the result is exact for
# every input and does not depend on `--overflow-checks`.
.globl __rmc_rotate_left
.type __rmc_rotate_left, @function
__rmc_rotate_left:
    mov %rdi, %rax           # value
    mov %rsi, %rcx           # count (masked to 6 bits by the instruction)
    rol %cl, %rax
    ret
.size __rmc_rotate_left, .-__rmc_rotate_left
"#,
        defines: &["__rmc_rotate_left"],
    },
    Piece {
        text: r#"
.text
# long __rmc_rotate_right(long x, long n)  -- the bits rotated right by `n`. The
# mirror of `__rmc_rotate_left`; `ror %cl` masks the count to 6 bits.
.globl __rmc_rotate_right
.type __rmc_rotate_right, @function
__rmc_rotate_right:
    mov %rdi, %rax           # value
    mov %rsi, %rcx           # count (masked to 6 bits by the instruction)
    ror %cl, %rax
    ret
.size __rmc_rotate_right, .-__rmc_rotate_right
"#,
        defines: &["__rmc_rotate_right"],
    },
    Piece {
        text: r#"
.text
# long __rmc_wrapping_add(long a, long b)  -- two's complement wrapping,
# identical to the bare `add` the operator path emits. It exists so
# `a.wrapping_add(b)` has the same operand evaluation (left to right, each once)
# as a binary operator, without a `BinOp` variant.
.globl __rmc_wrapping_add
.type __rmc_wrapping_add, @function
__rmc_wrapping_add:
    mov %rdi, %rax
    add %rsi, %rax
    ret
.size __rmc_wrapping_add, .-__rmc_wrapping_add
"#,
        defines: &["__rmc_wrapping_add"],
    },
    Piece {
        text: r#"
.text
# long __rmc_wrapping_sub(long a, long b)  -- `a - b`, wrapping, as above.
.globl __rmc_wrapping_sub
.type __rmc_wrapping_sub, @function
__rmc_wrapping_sub:
    mov %rdi, %rax
    sub %rsi, %rax
    ret
.size __rmc_wrapping_sub, .-__rmc_wrapping_sub
"#,
        defines: &["__rmc_wrapping_sub"],
    },
    Piece {
        text: r#"
.text
# long __rmc_saturating_add(long a, long b)  -- `a + b`, clamped to `i64::MIN`
# / `i64::MAX` instead of wrapping. `add` sets `OF` exactly when the signed sum
# does not fit, so the overflowed result is replaced by the bound the sign of
# `a` selects: a positive `a` overflowed upward, a negative one downward.
.globl __rmc_saturating_add
.type __rmc_saturating_add, @function
__rmc_saturating_add:
    mov %rdi, %rax
    add %rsi, %rax
    jno .Lsatadd_done
    test %rdi, %rdi
    movabs $9223372036854775807, %rax
    jns .Lsatadd_done
    movabs $-9223372036854775808, %rax
.Lsatadd_done:
    ret
.size __rmc_saturating_add, .-__rmc_saturating_add
"#,
        defines: &["__rmc_saturating_add"],
    },
    Piece {
        text: r#"
.text
# long __rmc_saturating_sub(long a, long b)  -- `a - b`, clamped. `sub` sets
# `OF` when the difference does not fit; the sign of `a` picks the bound (a
# positive `a` overflowed upward, a negative one downward), exactly as for the
# sum.
.globl __rmc_saturating_sub
.type __rmc_saturating_sub, @function
__rmc_saturating_sub:
    mov %rdi, %rax
    sub %rsi, %rax
    jno .Lsatsub_done
    test %rdi, %rdi
    movabs $9223372036854775807, %rax
    jns .Lsatsub_done
    movabs $-9223372036854775808, %rax
.Lsatsub_done:
    ret
.size __rmc_saturating_sub, .-__rmc_saturating_sub
"#,
        defines: &["__rmc_saturating_sub"],
    },
    Piece {
        text: r#"
.text
# long __rmc_saturating_mul(long a, long b)  -- `a * b`, clamped. `imul` sets
# `OF` exactly when the product does not fit. When it does not, the true product
# is positive when the operands share a sign and negative otherwise, so the
# bound is chosen by the sign of `a ^ b`: a clear sign bit means the same sign,
# hence `i64::MAX`.
.globl __rmc_saturating_mul
.type __rmc_saturating_mul, @function
__rmc_saturating_mul:
    mov %rdi, %rax
    imul %rsi, %rax
    jno .Lsatsmul_done
    mov %rdi, %rcx
    xor %rsi, %rcx
    movabs $9223372036854775807, %rax
    jns .Lsatsmul_done
    movabs $-9223372036854775808, %rax
.Lsatsmul_done:
    ret
.size __rmc_saturating_mul, .-__rmc_saturating_mul
"#,
        defines: &["__rmc_saturating_mul"],
    },
    Piece {
        text: r#"
.text
# long __rmc_pow(long base, long exp)  -- `base` raised to `exp` by binary
# exponentiation, in wrapping two's-complement arithmetic (the product is
# unchecked, so `i64::MIN` and other overflow cases wrap as `rustc -O` does).
# A negative `exp` has no `i64` result, so the loop is skipped and `1` is
# returned, matching the constant `0` exponent's value.
.globl __rmc_pow
.type __rmc_pow, @function
__rmc_pow:
    mov $1, %rax             # result
    test %rsi, %rsi
    jle .Lpow_done
.Lpow_loop:
    mov %rsi, %rcx
    and $1, %ecx             # the low bit of the exponent
    jz .Lpow_skip
    imul %rdi, %rax
.Lpow_skip:
    imul %rdi, %rdi          # square the base
    shr $1, %rsi             # drop the bit just used
    jnz .Lpow_loop
.Lpow_done:
    ret
.size __rmc_pow, .-__rmc_pow
"#,
        defines: &["__rmc_pow"],
    },
];

/// The identifiers in `src`, with `#`-to-end-of-line comments dropped. Matching
/// on whole identifiers keeps `free` from matching inside `__rmc_freelist`,
/// `alloc` from matching inside `alloc_big`, and a word in prose (the "free
/// list" in `alloc`'s comment) from counting as a symbol reference.
fn idents(src: &str) -> HashSet<&str> {
    let b = src.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'_' || c.is_ascii_alphanumeric() {
            let start = i;
            while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            out.insert(&src[start..i]);
        } else {
            i += 1;
        }
    }
    out
}

/// The identifiers of each piece, computed once per process. A piece's text
/// never changes, so its scan is not worth repeating per compile.
fn piece_idents() -> &'static [HashSet<&'static str>] {
    static CACHE: OnceLock<Vec<HashSet<&'static str>>> = OnceLock::new();
    CACHE.get_or_init(|| PIECES.iter().map(|p| idents(p.text)).collect())
}

/// The global symbols `user_asm` defines, so a piece that redefines one is
/// skipped. A symbol is defined by a label at column 0 (`name:`) or by a
/// `.globl`/`.global` directive.
fn defined_symbols(src: &str) -> HashSet<&str> {
    let mut out = HashSet::new();
    for line in src.lines() {
        if let Some(rest) =
            line.strip_prefix(".globl ").or_else(|| line.strip_prefix(".global "))
        {
            let name = rest.trim();
            if !name.is_empty() {
                out.insert(name);
            }
            continue;
        }
        if line.starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric()) {
            if let Some(name) = line.strip_suffix(':') {
                if !name.is_empty()
                    && name.bytes().all(|b| b == b'_' || b.is_ascii_alphanumeric())
                {
                    out.insert(name);
                }
            }
        }
    }
    out
}

/// The subset of the runtime that `user_asm` reaches.
///
/// `_start` is always kept — it is the process entry — and every piece it pulls
/// in follows. A piece is kept when the program names one of its symbols, and
/// keeping a piece can name more symbols, so the selection runs to a fixpoint.
/// A program that never allocates does not carry the allocator, and one that
/// never indexes an array does not carry the bounds trap.
/// A program may shim a runtime helper with an ordinary function (`fn
/// alloc(..)`) so the same source also builds under rustc; emitting the
/// runtime's own `alloc` too would be a duplicate symbol.
pub fn for_program(user_asm: &str) -> String {
    // The global symbols the program defines itself. A piece that would define
    // one of them is skipped, so a user shim of a built-in wins.
    let user_defs = defined_symbols(user_asm);
    // Tokenise the program once and look each symbol up in the set, rather than
    // rescanning the whole text per symbol: the scan is O(text * symbols) and
    // was the single largest cost in a compile. The piece texts are static, so
    // their token sets are cached across calls.
    let pids = piece_idents();
    let uids = idents(user_asm);
    let all: Vec<&'static str> = PIECES.iter().flat_map(|p| p.defines.iter().copied()).collect();

    // Seed: `_start` is the process entry and is always kept, plus any runtime
    // symbol the program names directly.
    let mut wanted = vec![false; all.len()];
    for (j, d) in all.iter().enumerate() {
        if *d == "_start" || uids.contains(d) {
            wanted[j] = true;
        }
    }

    // For each piece, the symbols it names. This relation is fixed, so it is
    // derived from the cached token sets rather than rescanned.
    let names: Vec<Vec<usize>> = pids
        .iter()
        .map(|set| (0..all.len()).filter(|&j| set.contains(all[j])).collect())
        .collect();

    // Fixpoint: keep every piece that defines a wanted symbol, then mark the
    // symbols each kept piece names.
    let mut included = vec![false; PIECES.len()];
    let mut dirty = true;
    while dirty {
        dirty = false;
        for (i, p) in PIECES.iter().enumerate() {
            if included[i] {
                continue;
            }
            // A piece the program redefines is left out, so the user's own
            // `alloc`/`free` shim is the only definition linked.
            if p.defines.iter().any(|d| user_defs.contains(d)) {
                continue;
            }
            let defines_wanted = p
                .defines
                .iter()
                .any(|d| all.iter().position(|x| x == d).is_some_and(|j| wanted[j]));
            if !defines_wanted {
                continue;
            }
            included[i] = true;
            dirty = true;
            for &j in &names[i] {
                wanted[j] = true;
            }
        }
    }

    let mut out = String::new();
    for (i, p) in PIECES.iter().enumerate() {
        if included[i] {
            out.push_str(p.text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents_matches_whole_symbols_only() {
        assert!(idents("call print_i64").contains("print_i64"));
        assert!(idents("print_i64:").contains("print_i64"));
        // `free` must not match inside `__rmc_freelist`.
        assert!(!idents("__rmc_freelist: .quad 0").contains("free"));
        assert!(!idents("call alloc_big").contains("alloc"));
        // A trailing/leading non-identifier boundary counts.
        assert!(idents("(free)").contains("free"));
        // A comment is dropped, but only to end of line.
        assert!(!idents("# print_i64\n\tret").contains("print_i64"));
        assert!(idents("# x\n\tcall print_i64").contains("print_i64"));
    }

    #[test]
    fn a_bare_main_keeps_only_the_entry() {
        // No runtime call: only `_start` (and the argv globals it writes) are
        // linked.
        let r = for_program(".globl main\nmain:\n\tmov $0, %rax\n\tret\n");
        assert!(r.contains("_start:"), "entry point kept");
        assert!(r.contains("__rmc_argc:"), "argv globals kept for _start");
        assert!(!r.contains("print_i64:"), "printer dropped:\n{r}");
        assert!(!r.contains("alloc:"), "allocator dropped:\n{r}");
        assert!(!r.contains("__rmc_bounds:"), "bounds trap dropped:\n{r}");
    }

    #[test]
    fn a_call_pulls_in_its_piece() {
        let r = for_program("main:\n\tcall print_i64\n\tret\n");
        assert!(r.contains("print_i64:"), "printer kept");
        assert!(!r.contains("print_str:"), "unused printer dropped");
        assert!(!r.contains("alloc:"), "allocator dropped");
    }

    #[test]
    fn alloc_pulls_in_the_arena_globals() {
        let r = for_program("main:\n\tcall alloc\n\tret\n");
        assert!(r.contains("alloc:"), "allocator kept");
        assert!(r.contains("__rmc_arena:"), "arena globals kept for alloc");
        assert!(r.contains("__rmc_freelist:"), "free list kept for alloc");
    }

    #[test]
    fn the_free_list_global_does_not_pull_in_free() {
        // `alloc` reads `__rmc_freelist`; that must not drag in the `free`
        // function, which merely shares a name prefix.
        let r = for_program("main:\n\tcall alloc\n\tret\n");
        assert!(!r.contains("free:"), "free dropped despite __rmc_freelist:\n{r}");
    }

    #[test]
    fn a_user_definition_suppresses_the_runtime_piece() {
        // A program that shims `alloc`/`free` (so the same source also builds
        // under rustc) must not get the runtime's own copies too: the two would
        // be duplicate symbols at link time.
        let r = for_program(
            ".globl alloc\nalloc:\n\tmov $0, %rax\n\tret\n\
             .globl free\nfree:\n\tret\n\
             main:\n\tcall alloc\n\tcall free\n\tret\n",
        );
        assert!(!r.contains("\nalloc:"), "runtime alloc dropped:\n{r}");
        assert!(!r.contains("\nfree:"), "runtime free dropped:\n{r}");
        // The arena globals are only needed by the runtime allocator, which is
        // gone, so they go too.
        assert!(!r.contains("__rmc_arena:"), "arena globals dropped:\n{r}");
    }

    #[test]
    fn every_piece_is_reachable_from_some_symbol() {
        // A piece no program could ever pull in would be dead weight in the
        // table; make sure each one defines at least one symbol.
        for p in PIECES {
            assert!(!p.defines.is_empty(), "a piece with no symbols");
        }
    }
}
