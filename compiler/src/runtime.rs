//! Minimal freestanding runtime, written directly in x86-64 assembly.
//!
//! There is no libc: process start-up, integer/string printing, memory
//! allocation and exit all go through raw Linux syscalls. This keeps the
//! produced binaries tiny and independent of any C runtime.

pub const RUNTIME_ASM: &str = r#"
.data
.align 8
__rmc_argc: .quad 0
__rmc_argv: .quad 0
# First free block, or 0. The free list is address-ordered so neighbouring
# blocks can be coalesced on free.
__rmc_freelist: .quad 0
# Bump pointer and end of the current mmap'd chunk. Fresh blocks are carved
# from here; freed blocks go back on the free list for reuse.
__rmc_arena: .quad 0
__rmc_arena_end: .quad 0

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

# void exit(long code)
.globl exit
.type exit, @function
exit:
    mov $60, %rax
    syscall
.size exit, .-exit

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

# long alloc(long n)  -- first-fit block from the free list, else carve from the
# arena, else mmap a fresh chunk. Returns a pointer with room for `n` bytes.
# Layout of a block: [size: 8 bytes][payload ...]; the pointer handed out points
# at the payload, and the header sits 8 bytes before it.
.globl alloc
.type alloc, @function
alloc:
    push %rbp
    mov %rsp, %rbp
    push %r12
    push %r13
    push %r14
    push %r15
    # Round the request up to a whole number of 8-byte words.
    add $7, %rdi
    and $-8, %rdi
    mov %rdi, %r12           # requested payload size
    test %r12, %r12
    jnz .Lalloc_sz_ok
    mov $8, %r12             # never hand out a zero-size block
.Lalloc_sz_ok:
    mov %r12, %r13           # total size including the 8-byte header
    add $8, %r13

    # Walk the free list for the first block large enough (first fit).
    lea __rmc_freelist(%rip), %r14   # pointer to the link we are examining
    mov (%r14), %rax
.Lalloc_scan:
    test %rax, %rax
    jz .Lalloc_from_arena
    mov (%rax), %rcx         # block total size
    cmp %r13, %rcx
    jae .Lalloc_found
    lea 8(%rax), %r14        # next link lives at block+8
    mov (%r14), %rax
    jmp .Lalloc_scan
.Lalloc_found:
    mov 8(%rax), %rdx        # successor link
    mov %rdx, (%r14)
    lea 8(%rax), %rax        # payload
    jmp .Lalloc_done

.Lalloc_from_arena:
    mov __rmc_arena(%rip), %rax
    mov __rmc_arena_end(%rip), %rdx
    mov %rdx, %rcx
    sub %rax, %rcx           # bytes left in the current chunk
    cmp %r13, %rcx
    jae .Lalloc_carve
    # Need a new chunk. Map max(1 MiB, 2 * request), page rounded.
    mov $1048576, %rsi
    lea (%r13,%r13), %rcx
    cmp %rsi, %rcx
    jbe .Lalloc_have_size
    mov %rcx, %rsi
.Lalloc_have_size:
    add $4095, %rsi
    and $-4096, %rsi
    mov %rsi, %r15           # chunk length
    xor %rdi, %rdi
    mov $3, %rdx             # PROT_READ|PROT_WRITE
    mov $0x22, %r10          # MAP_PRIVATE|MAP_ANONYMOUS
    mov $-1, %r8
    xor %r9, %r9
    mov $9, %rax             # mmap
    syscall
    cmp $-4095, %rax
    jae .Lalloc_fail
    mov %rax, __rmc_arena(%rip)
    add %r15, %rax
    mov %rax, __rmc_arena_end(%rip)
    mov __rmc_arena(%rip), %rax
.Lalloc_carve:
    mov %r13, (%rax)         # header
    lea 8(%rax), %rdx
    add %r13, %rax
    mov %rax, __rmc_arena(%rip)
    mov %rdx, %rax           # payload
    jmp .Lalloc_done

.Lalloc_done:
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    leave
    ret
.Lalloc_fail:
    xor %eax, %eax
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    leave
    ret
.size alloc, .-alloc

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

# void __rmc_overflow(void)  -- a checked +,-,* overflowed; write a fixed
# message and exit with status 101, mirroring a Rust panic.
.globl __rmc_overflow
.type __rmc_overflow, @function
__rmc_overflow:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_overflow_msg(%rip), %rsi
    mov $25, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_overflow, .-__rmc_overflow

.section .rodata
__rmc_overflow_msg: .ascii "rmc: arithmetic overflow\n"

# void __rmc_bounds(void)  -- a checked array index was out of range; write a
# fixed message and exit with status 101, mirroring a Rust panic.
.globl __rmc_bounds
.type __rmc_bounds, @function
__rmc_bounds:
    mov $1, %rax
    mov $2, %rdi             # stderr, like a Rust panic
    lea __rmc_bounds_msg(%rip), %rsi
    mov $30, %rdx
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_bounds, .-__rmc_bounds

.section .rodata
__rmc_bounds_msg: .ascii "rmc: index out of bounds\n"
"#;
