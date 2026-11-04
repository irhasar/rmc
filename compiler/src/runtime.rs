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
];

/// True when `needle` appears in `haystack` as a whole symbol: the bytes on
/// either side are not identifier characters. Comments are stripped first, so a
/// word in prose (the "free list" in `alloc`'s comment) is not mistaken for a
/// symbol reference. This also keeps `free` from matching inside
/// `__rmc_freelist` and `alloc` from matching inside a longer name.
fn mentions(haystack: &str, needle: &str) -> bool {
    let n = needle.as_bytes();
    if n.is_empty() {
        return false;
    }
    let ident = |b: u8| b == b'_' || b.is_ascii_alphanumeric();
    for line in haystack.lines() {
        // `#` starts a comment on x86 GNU as; drop it and everything after.
        let code = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        let h = code.as_bytes();
        if n.len() > h.len() {
            continue;
        }
        if (0..=h.len() - n.len()).any(|i| {
            &h[i..i + n.len()] == n
                && (i == 0 || !ident(h[i - 1]))
                && (i + n.len() == h.len() || !ident(h[i + n.len()]))
        }) {
            return true;
        }
    }
    false
}

/// The subset of the runtime that `user_asm` reaches.
///
/// `_start` is always kept — it is the process entry — and every piece it pulls
/// in follows. A piece is kept when the program names one of its symbols, and
/// keeping a piece can name more symbols, so the selection runs to a fixpoint.
/// A program that never allocates does not carry the allocator, and one that
/// never indexes an array does not carry the bounds trap.
pub fn for_program(user_asm: &str) -> String {
    let mut wanted: Vec<&'static str> = vec!["_start"];
    fn add(name: &'static str, wanted: &mut Vec<&'static str>) {
        if !wanted.contains(&name) {
            wanted.push(name);
        }
    }
    // Seed: any runtime symbol the program names directly.
    for p in PIECES {
        for d in p.defines {
            if mentions(user_asm, d) {
                add(d, &mut wanted);
            }
        }
    }
    // Fixpoint: keep every piece that defines a wanted symbol, then add the
    // symbols each kept piece names.
    let mut included = vec![false; PIECES.len()];
    loop {
        let mut changed = false;
        for (i, p) in PIECES.iter().enumerate() {
            if included[i] || !p.defines.iter().any(|d| wanted.contains(d)) {
                continue;
            }
            included[i] = true;
            changed = true;
            for q in PIECES {
                for d in q.defines {
                    if mentions(p.text, d) {
                        add(d, &mut wanted);
                    }
                }
            }
        }
        if !changed {
            break;
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
    fn mentions_matches_whole_symbols_only() {
        assert!(mentions("call print_i64", "print_i64"));
        assert!(mentions("print_i64:", "print_i64"));
        // `free` must not match inside `__rmc_freelist`.
        assert!(!mentions("__rmc_freelist: .quad 0", "free"));
        assert!(!mentions("call alloc_big", "alloc"));
        // A trailing/leading non-identifier boundary counts.
        assert!(mentions("(free)", "free"));
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
    fn every_piece_is_reachable_from_some_symbol() {
        // A piece no program could ever pull in would be dead weight in the
        // table; make sure each one defines at least one symbol.
        for p in PIECES {
            assert!(!p.defines.is_empty(), "a piece with no symbols");
        }
    }
}
