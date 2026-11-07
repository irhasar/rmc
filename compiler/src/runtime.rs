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
# void __rmc_write(long ptr, long len)  -- write `len` bytes at `ptr` to
# stdout, no newline. This is the primitive the `core::fmt` lowering uses to
# emit a literal run; unlike `print_str` it neither prefixes a length word nor
# appends a newline.
.globl __rmc_write
.type __rmc_write, @function
__rmc_write:
    mov %rsi, %rdx           # count
    mov %rdi, %rsi           # buffer
    mov $1, %rdi             # fd = stdout
    mov $1, %rax             # SYS_write
    syscall
    ret
.size __rmc_write, .-__rmc_write
"#,
        defines: &["__rmc_write"],
    },
    Piece {
        text: r#"
# void __rmc_print(long a)  -- the sink for `print!`/`println!`. `a` is the
# address of a `core::fmt::Arguments` {template: ptr, args: ptr}. When the low
# bit of `args` is set the value came from `Arguments::from_str`, which is what
# rustc folds a format string with no runtime arguments to: `template` points
# at the bytes and `args >> 1` is their length, so they are written verbatim
# (the macro's trailing newline is already in them). Otherwise `template` is a
# `format_args!` template and `args` an array of `Argument`s, one per
# placeholder; the runtime walks the template and, for each placeholder, calls
# the formatter the argument carries with a pointer to its value.
.globl __rmc_print
.type __rmc_print, @function
__rmc_print:
    mov $1, %r8              # fd = stdout
    jmp __rmc_write_args
.size __rmc_print, .-__rmc_print

.globl __rmc_eprint
.type __rmc_eprint, @function
__rmc_eprint:
    mov $2, %r8              # fd = stderr
    jmp __rmc_write_args
.size __rmc_eprint, .-__rmc_eprint

__rmc_write_args:
    mov (%rdi), %rsi         # template pointer
    mov 8(%rdi), %rdx        # args pointer, or len<<1|1 for `from_str`
    mov %rdx, %rax
    and $1, %rax             # low bit set => `from_str` (no placeholders)
    jnz __rmc_write_from_str
    jmp __rmc_write_template

# `Arguments::from_str`: `%rsi` points at rmc's `{len: i64, bytes..}` string
# record (rmc models `&str` as one word, the record address) and `args >> 1` is
# the byte length. Write the bytes, skipping the record's length word.
__rmc_write_from_str:
    mov %rdx, %rcx
    shr $1, %rcx             # length
    mov %rcx, %rdx           # count
    add $8, %rsi             # -> the bytes
    mov %r8, %rdi            # fd
    mov $1, %rax             # SYS_write
    syscall
    ret

# A `format_args!` template: walk it, writing literal runs and expanding each
# placeholder. `%rsi` = template cursor, `%r9` = argument cursor, `%r8` = fd.
__rmc_write_template:
    push %rbp
    mov %rsp, %rbp
    push %r12                # template cursor
    push %r13                # argument cursor
    push %r14                # fd
    push %r15                # next implicit argument index
    mov %rsi, %r12
    mov %rdx, %r13
    mov %r8, %r14
    xor %r15, %r15
.Ltmpl_loop:
    movzbl (%r12), %eax
    test %eax, %eax
    jz .Ltmpl_done
    cmp $0x80, %eax
    jae .Ltmpl_high
    # Literal piece: `len` bytes follow the length byte.
    inc %r12
    mov %r14, %rdi
    mov %r12, %rsi
    mov %rax, %rdx
    add %rax, %r12
    mov $1, %rax
    syscall
    jmp .Ltmpl_loop
.Ltmpl_high:
    cmp $0xC0, %eax
    jae .Ltmpl_placeholder
    # Long literal: 0x80, u16 length, then the bytes.
    movzbl 1(%r12), %eax
    movzbl 2(%r12), %ecx
    shl $8, %ecx
    or %ecx, %eax
    add $3, %r12
    mov %r14, %rdi
    mov %r12, %rsi
    mov %rax, %rdx
    add %rax, %r12
    mov $1, %rax
    syscall
    jmp .Ltmpl_loop
.Ltmpl_placeholder:
    # 0b11xxxxxx: optional fields follow, each present when its bit is set --
    # bit 0 flags (u32), bit 1 width (u16), bit 2 precision (u16), bit 3
    # arg_index (u16), bit 4 width indirect, bit 5 precision indirect. The
    # formatters must render flags, width and precision, but their signature is
    # fixed (`fn(&T)`), so record the placeholder's options in the global
    # `__rmc_fmt_mode` before the call: bits 0..31 are the flags word (fill in
    # bits 0..20, sign/alternate/alignment above), width in bits 32..47 and
    # precision in bits 48..63. A placeholder with no options leaves it 0,
    # which lets every formatter take its fast path.
    mov %eax, %edx           # the flag byte
    inc %r12
    xor %r10, %r10
    test $0x01, %dl          # flags present
    jz .Lph_no_flags
    mov (%r12), %r10d
    add $4, %r12
.Lph_no_flags:
    test $0x02, %dl          # width present
    jz .Lph_no_width
    movzwl (%r12), %eax
    test $0x10, %dl          # width indirect: the field is an argument index
    jz .Lph_width_val
    shl $4, %rax             # the Count value sits at +8 in the argument
    add %r13, %rax
    movzwl 8(%rax), %eax
.Lph_width_val:
    shl $32, %rax
    or %rax, %r10
    add $2, %r12
.Lph_no_width:
    test $0x04, %dl          # precision present
    jz .Lph_no_prec
    movzwl (%r12), %eax
    test $0x20, %dl          # precision indirect
    jz .Lph_prec_val
    shl $4, %rax
    add %r13, %rax
    movzwl 8(%rax), %eax
.Lph_prec_val:
    inc %eax                 # stored as prec + 1, so 0 still means "none"
    shl $48, %rax
    or %rax, %r10
    add $2, %r12
.Lph_no_prec:
    # A precision of 0 has no precision field, only the flag, so re-create it.
    mov %r10, %rax
    shr $48, %rax
    jnz .Lph_have_prec
    test $0x10000000, %r10d  # PRECISION_FLAG
    jz .Lph_have_prec
    movabs $0x0001000000000000, %rax
    or %rax, %r10
.Lph_have_prec:
    # A precision wider than the widest rendered value (96 bytes) is unbounded.
    mov %r10, %rax
    shr $48, %rax
    cmp $96, %rax
    jbe .Lph_prec_ok
    movabs $0x0000FFFFFFFFFFFF, %rcx
    and %rcx, %r10
.Lph_prec_ok:
    test $0x08, %dl          # arg_index present
    jz .Lph_implicit
    movzwl (%r12), %eax
    add $2, %r12
    jmp .Lph_have_index
.Lph_implicit:
    mov %r15, %rax           # next argument, in order
    inc %r15
.Lph_have_index:
    # Argument i is 16 bytes: value pointer at +0, formatter at +8. Resolve it
    # before publishing the mode, which clobbers the caller-saved registers.
    shl $4, %rax
    add %r13, %rax
    mov %rax, %r11
    mov %r10, %rdi
    call __rmc_fmt_set
    mov (%r11), %rdi         # value pointer
    call *8(%r11)            # formatter(value)
    jmp .Ltmpl_loop
.Ltmpl_done:
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    ret
"#,
        defines: &["__rmc_print", "__rmc_eprint"],
    },
    Piece {
        text: r#"
# void __rmc_panic(long msg)  -- the `panic!`/`todo!`/`unreachable!` sink. A
# freestanding program has no unwinder, so it writes a fixed diagnostic to
# stderr and exits non-zero, which is the observable behaviour of
# `panic=abort`. The message argument is ignored: the `&str`/`Arguments` it
# carries would drag in the whole `core::fmt` machinery for text nothing
# checks.
.globl __rmc_panic
.type __rmc_panic, @function
__rmc_panic:
    mov $2, %rdi             # fd = stderr
    lea __rmc_panic_msg(%rip), %rsi
    mov $9, %rdx             # "panicked\n"
    mov $1, %rax
    syscall
    mov $101, %rdi
    mov $60, %rax
    syscall
.size __rmc_panic, .-__rmc_panic

.section .rodata
__rmc_panic_msg: .ascii "panicked\n"
"#,
        defines: &["__rmc_panic"],
    },
    Piece {
        text: r#"
# int __rmc_compare_bytes(const u8 *a, const u8 *b, usize n)  -- the
# `compare_bytes` intrinsic `str`/`[u8]` equality lowers to. rmc represents a
# string literal as the address of a `{len: i64, bytes..}` record and the
# `&[u8]` cast of one as the record's address, so the pointers the caller
# passes are record addresses, not byte pointers: skip each record's length
# word. Returns a negative, zero or positive int, so the caller's `== 0`
# test is the equality.
.globl __rmc_compare_bytes
.type __rmc_compare_bytes, @function
__rmc_compare_bytes:
    add $8, %rdi
    add $8, %rsi
    xor %rcx, %rcx
.Lcb_loop:
    cmp %rdx, %rcx
    jae .Lcb_equal
    movzbl (%rdi,%rcx), %eax
    movzbl (%rsi,%rcx), %r8d
    sub %r8d, %eax
    jnz .Lcb_done
    inc %rcx
    jmp .Lcb_loop
.Lcb_equal:
    xor %eax, %eax
.Lcb_done:
    ret
.size __rmc_compare_bytes, .-__rmc_compare_bytes
"#,
        defines: &["__rmc_compare_bytes"],
    },
    Piece {
        text: r#"
# The `core::fmt` formatters. `format_args!` reifies one per placeholder and
# the `Argument` array carries its address; the template walker calls it with a
# pointer to the value. Each writes to stdout with a raw syscall. They are kept
# only when the program actually formats a runtime argument.
#
# The walker publishes each placeholder's flags, width and precision in
# `__rmc_fmt_mode` before the call (0 for a plain placeholder, so the common
# case skips all of this). `__rmc_fmt_pad` then writes the value's digits with
# the fill, sign, prefix, alignment and zero padding those options ask for.
.data
.align 8
__rmc_fmt_mode: .quad 0

.text
# long __rmc_utf8_trunc(const char *s, long n, long max)  -- the byte length of
# the first `max` UTF-8 characters of the `n`-byte string `s`.
__rmc_utf8_trunc:
    xor %rax, %rax
    test %rsi, %rsi
    jz .Lut_done
.Lut_loop:
    cmp %rsi, %rax
    jae .Lut_done
    test %rdx, %rdx
    jz .Lut_done
    movzbl (%rdi,%rax), %ecx
    inc %rax
    cmp $0x80, %ecx
    jb .Lut_next
    cmp $0xE0, %ecx
    jae .Lut_3
    inc %rax
    jmp .Lut_next
.Lut_3:
    cmp $0xF0, %ecx
    jae .Lut_4
    add $2, %rax
    jmp .Lut_next
.Lut_4:
    add $3, %rax
.Lut_next:
    dec %rdx
    jmp .Lut_loop
.Lut_done:
    ret
.size __rmc_utf8_trunc, .-__rmc_utf8_trunc

# void __rmc_fmt_set(long mode)  -- stash the current placeholder's options:
# bits 0..20 the fill char, 21..31 the sign/alternate/zero-pad/align flags
# (rustc's `FormattingOptions::flags`), 32..47 the width, 48..63 the precision.
.globl __rmc_fmt_set
.type __rmc_fmt_set, @function
__rmc_fmt_set:
    mov %rdi, __rmc_fmt_mode(%rip)
    ret
.size __rmc_fmt_set, .-__rmc_fmt_set

# long __rmc_fmt_pad(const char *digits, long len, const char *buf_end)  --
# write the pending value honouring `__rmc_fmt_mode`. `%rdi` points at the
# digits (sign, `0x`/`0b`/`0o` prefix and body already in place), `%rsi` is
# their length and `%rdx` is the end of the 96-byte scratch buffer they were
# built in. With no options set this is a bare write; otherwise the value is
# padded to the width.
# void __rmc_put_fill(char c, long n)  -- write `n` copies of `c`. The fill can
# be wider than the scratch buffer, so it goes out in 128-byte chunks.
__rmc_put_fill:
    push %rbx
    push %r12
    push %r13
    mov %rsi, %r12
    mov %rdi, %rbx
    sub $128, %rsp
    mov %rsp, %rdi
    mov $128, %rcx
    mov %bl, %al
.Lpf_fill:
    mov %al, (%rdi)
    inc %rdi
    dec %rcx
    jnz .Lpf_fill
.Lpf_loop:
    test %r12, %r12
    jz .Lpf_done
    mov %r12, %rdx
    cmp $128, %rdx
    jbe .Lpf_chunk
    mov $128, %rdx
.Lpf_chunk:
    sub %rdx, %r12
    mov %rsp, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    jmp .Lpf_loop
.Lpf_done:
    add $128, %rsp
    pop %r13
    pop %r12
    pop %rbx
    ret
.size __rmc_put_fill, .-__rmc_put_fill

# void __rmc_fmt_pad(const char *value, long len, long text)  -- write the
# pending value honouring `__rmc_fmt_mode`. `%rdi`/`%rsi` are the value's bytes
# (sign and any `0x`/`0b`/`0o` prefix already in place) and `%rdx` is non-zero
# when the value reads as text, the only case where a precision truncates. With
# no options set this is a bare write; otherwise the value is padded to width.
__rmc_fmt_pad:
    cmpq $0, __rmc_fmt_mode(%rip)
    jne .Lpad_slow
    mov %rsi, %rdx
    mov %rdi, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    ret
.Lpad_slow:
    push %rbx
    push %rbp
    push %r12
    push %r13
    push %r14
    push %r15
    mov %rdi, %rbx           # value start
    mov %rsi, %rbp           # len
    # A precision truncates a text value to that many characters; a numeric
    # value ignores it, exactly as rustc does.
    test %rdx, %rdx
    jz .Lprec_done
    mov __rmc_fmt_mode(%rip), %rdx
    shr $48, %rdx
    test %rdx, %rdx
    jz .Lprec_done
    dec %rdx
    mov %rbx, %rdi
    mov %rbp, %rsi
    call __rmc_utf8_trunc
    mov %rax, %rbp
.Lprec_done:
    mov __rmc_fmt_mode(%rip), %r14
    mov %r14, %r15
    shr $32, %r15
    and $0xffff, %r15        # width
    mov %r15, %r13
    sub %rbp, %r13           # padding
    jbe .Lpad_value
    # Zero padding: the `0` flag (SIGN_AWARE_ZERO_PAD), no explicit `-`, and a
    # width. Insert the zeros between the sign/prefix and the body.
    test $0x1000000, %r14d
    jz .Lpad_align
    test $0x400000, %r14d
    jnz .Lpad_align
    mov %rbx, %r10           # prefix end: past the sign and any base prefix
    test %rbp, %rbp
    jz .Lzp_go
    movzbl (%r10), %eax
    cmp $45, %al             # '-'
    je .Lzp_sign
    cmp $43, %al             # '+'
    je .Lzp_sign
    cmp $32, %al             # ' '
    je .Lzp_sign
    jmp .Lzp_pre
.Lzp_sign:
    inc %r10
.Lzp_pre:
    mov %rbx, %rdx
    add %rbp, %rdx
    sub %r10, %rdx
    cmp $2, %rdx
    jb .Lzp_go
    movzbl (%r10), %eax
    cmp $48, %al             # '0'
    jne .Lzp_go
    movzbl 1(%r10), %eax
    cmp $120, %al            # 'x'
    je .Lzp_two
    cmp $88, %al             # 'X'
    je .Lzp_two
    cmp $98, %al             # 'b'
    je .Lzp_two
    cmp $111, %al            # 'o'
    je .Lzp_two
    jmp .Lzp_go
.Lzp_two:
    add $2, %r10
.Lzp_go:
    mov %r10, %rdx
    sub %rbx, %rdx           # prefix length
    jz .Lzp_zeros
    mov %rbx, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
.Lzp_zeros:
    mov $48, %edi
    mov %r13, %rsi
    call __rmc_put_fill
    mov %rbx, %rdx
    add %rbp, %rdx
    sub %r10, %rdx           # body length
    jz .Lpad_done
    mov %r10, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    jmp .Lpad_done
.Lpad_align:
    mov %r14, %r8
    and $0xff, %r8           # fill byte
    test %r8d, %r8d
    jnz .Lpad_have_fill
    mov $32, %r8d
.Lpad_have_fill:
    mov %r14, %rax
    shr $29, %rax
    and $3, %eax             # 0 left, 1 right, 2 center, 3 default
    cmp $2, %eax
    je .Lpad_center
    test %eax, %eax
    jz .Lpad_left
    # right, or the numeric default
    mov %r8d, %edi
    mov %r13, %rsi
    call __rmc_put_fill
    jmp .Lpad_value
.Lpad_center:
    mov %r13, %rax
    shr $1, %rax
    mov %rax, %r15           # left half
    mov %r8d, %edi
    mov %rax, %rsi
    call __rmc_put_fill
    mov %rbx, %rsi
    mov %rbp, %rdx
    mov $1, %rdi
    mov $1, %rax
    syscall
    mov %r13, %rsi
    sub %r15, %rsi           # right half
    mov %r8d, %edi
    call __rmc_put_fill
    jmp .Lpad_done
.Lpad_left:
    mov %rbx, %rsi
    mov %rbp, %rdx
    mov $1, %rdi
    mov $1, %rax
    syscall
    mov %r8d, %edi
    mov %r13, %rsi
    call __rmc_put_fill
    jmp .Lpad_done
.Lpad_value:
    mov %rbx, %rsi
    mov %rbp, %rdx
    mov $1, %rdi
    mov $1, %rax
    syscall
.Lpad_done:
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    pop %rbx
    ret
.size __rmc_fmt_pad, .-__rmc_fmt_pad

# void __rmc_emit_signed(long v)  -- `%rax` is the value and `%rdi` the base of
# a 96-byte buffer; write it in decimal, with the `+`/` ` sign flag when set.
__rmc_emit_signed:
    push %rbx
    push %r12
    mov %rdi, %rbx
    lea 96(%rbx), %r12
    xor %r8, %r8
    test %rax, %rax
    jns .Les_pos
    neg %rax
    mov $45, %r8             # '-'
    jmp .Les_abs
.Les_pos:
    mov __rmc_fmt_mode(%rip), %rdx
    test $0x200000, %edx     # SIGN_PLUS
    jz .Les_space
    mov $43, %r8             # '+'
    jmp .Les_abs
.Les_space:
    test $0x100000, %edx     # the ` ` flag
    jz .Les_abs
    mov $32, %r8             # ' '
.Les_abs:
    mov $10, %r9
.Les_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %r12
    mov %dl, (%r12)
    test %rax, %rax
    jnz .Les_loop
    test %r8, %r8
    jz .Les_pad
    dec %r12
    mov %r8b, (%r12)
.Les_pad:
    mov %r12, %rdi
    lea 96(%rbx), %rsi
    sub %r12, %rsi
    xor %edx, %edx           # a numeric value: a precision does not apply
    call __rmc_fmt_pad
    pop %r12
    pop %rbx
    ret

# void __rmc_emit_unsigned(long v)  -- `%rax` is the value, `%rdi` the buffer,
# `%r9` the base (2, 8, 10 or 16) and `%r10` 1 for a lowercase alphabet or 2 for
# uppercase. `{:#x}`/`{:#b}`/`{:#o}` add the base prefix.
__rmc_emit_unsigned:
    push %rbx
    push %r12
    push %r13
    mov %rdi, %rbx
    lea 96(%rbx), %r12
    mov %r10, %r13
    test %rax, %rax
    jnz .Leu_loop
    dec %r12
    movb $48, (%r12)
    jmp .Leu_prefix
.Leu_loop:
    xor %rdx, %rdx
    div %r9
    cmp $10, %dl
    jb .Leu_lt10
    add $87, %dl             # 10..15 -> 'a'..'f'
    cmp $2, %r13
    jne .Leu_digit
    sub $32, %dl             # -> 'A'..'F'
    jmp .Leu_digit
.Leu_lt10:
    add $48, %dl
.Leu_digit:
    dec %r12
    mov %dl, (%r12)
    test %rax, %rax
    jnz .Leu_loop
.Leu_prefix:
    mov __rmc_fmt_mode(%rip), %rdx
    test $0x800000, %edx     # ALTERNATE (the `#` flag)
    jz .Leu_sign
    cmp $16, %r9
    jne .Leu_poct
    dec %r12
    movb $120, (%r12)        # 'x' (lowercase, even for `{:#X}`)
    dec %r12
    movb $48, (%r12)
    jmp .Leu_sign
.Leu_poct:
    cmp $8, %r9
    jne .Leu_pbin
    dec %r12
    movb $111, (%r12)        # 'o'
    dec %r12
    movb $48, (%r12)
    jmp .Leu_sign
.Leu_pbin:
    dec %r12
    movb $98, (%r12)         # 'b'
    dec %r12
    movb $48, (%r12)
.Leu_sign:
    mov __rmc_fmt_mode(%rip), %rdx
    test $0x200000, %edx     # SIGN_PLUS: an unsigned value still gets a `+`
    jz .Leu_pad
    dec %r12
    movb $43, (%r12)
.Leu_pad:
    mov %r12, %rdi
    lea 96(%rbx), %rsi
    sub %r12, %rsi
    xor %edx, %edx           # a numeric value: a precision does not apply
    call __rmc_fmt_pad
    pop %r13
    pop %r12
    pop %rbx
    ret

# The entry points the `Argument`s carry. Each loads the value at its own
# width, reserves a scratch buffer and prints it with the pending options.
.globl __rmc_fmt_i64
.type __rmc_fmt_i64, @function
__rmc_fmt_i64:
    mov (%rdi), %rax
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_signed
    add $96, %rsp
    ret
.size __rmc_fmt_i64, .-__rmc_fmt_i64

.globl __rmc_fmt_u64
.type __rmc_fmt_u64, @function
__rmc_fmt_u64:
    mov (%rdi), %rax
    mov $10, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_u64, .-__rmc_fmt_u64

.globl __rmc_fmt_i32
.type __rmc_fmt_i32, @function
__rmc_fmt_i32:
    movslq (%rdi), %rax
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_signed
    add $96, %rsp
    ret
.size __rmc_fmt_i32, .-__rmc_fmt_i32

.globl __rmc_fmt_u32
.type __rmc_fmt_u32, @function
__rmc_fmt_u32:
    mov (%rdi), %eax
    mov $10, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_u32, .-__rmc_fmt_u32

.globl __rmc_fmt_bool
.type __rmc_fmt_bool, @function
__rmc_fmt_bool:
    call __rmc_fmt_left
    movzbl (%rdi), %eax
    sub $96, %rsp
    lea __rmc_fmt_false(%rip), %r8
    mov $5, %ecx
    test %eax, %eax
    jz .Lbool_copy
    lea __rmc_fmt_true(%rip), %r8
    mov $4, %ecx
.Lbool_copy:
    mov %rsp, %rdi
    xor %edx, %edx
.Lbool_cp_loop:
    cmp %ecx, %edx
    jae .Lbool_pad
    movzbl (%r8,%rdx), %eax
    mov %al, (%rdi,%rdx)
    inc %edx
    jmp .Lbool_cp_loop
.Lbool_pad:
    mov %ecx, %esi
    mov $1, %edx
    call __rmc_fmt_pad
    add $96, %rsp
    ret
.size __rmc_fmt_bool, .-__rmc_fmt_bool

.globl __rmc_fmt_char
.type __rmc_fmt_char, @function
__rmc_fmt_char:
    call __rmc_fmt_left
    mov (%rdi), %edi
    sub $96, %rsp
    mov %rsp, %rsi
    call __rmc_utf8_encode
    mov %rsp, %rdi
    mov %rax, %rsi
    mov $1, %edx
    call __rmc_fmt_pad
    add $96, %rsp
    ret
.size __rmc_fmt_char, .-__rmc_fmt_char

.globl __rmc_fmt_str
.type __rmc_fmt_str, @function
__rmc_fmt_str:
    # `%rdi` points at the `&str` slot the formatter was handed; the slot holds
    # the address of rmc's `{len: i64, bytes..}` record. Strings default to left
    # alignment, so when the placeholder asked for no alignment turn off the
    # numeric default before padding.
    mov (%rdi), %rdi
    call __rmc_fmt_left
    mov (%rdi), %rsi
    lea 8(%rdi), %rdi
    mov $1, %edx
    call __rmc_fmt_pad
    ret
.size __rmc_fmt_str, .-__rmc_fmt_str

# Set the left-alignment default for a placeholder whose value reads as text
# (`bool`, `char`, `&str`), unless one was asked for. `%rdi` is preserved.
__rmc_fmt_left:
    push %rdi
    mov __rmc_fmt_mode(%rip), %rax
    mov %rax, %rcx
    shr $29, %rcx
    and $3, %ecx
    cmp $3, %ecx
    jne .Lleft_done
    movabs $0xFFFFFFFF9FFFFFFF, %rdx
    and %rdx, %rax           # clear the align bits -> left
    mov %rax, __rmc_fmt_mode(%rip)
.Lleft_done:
    pop %rdi
    ret

.globl __rmc_fmt_str_debug
.type __rmc_fmt_str_debug, @function
# long __rmc_dbg_len(const char *s, long n)  -- the escaped length of the
# `n`-byte string `s`, excluding the surrounding quotes. `Debug` for `str`
# escapes `"`, `\\`, the three named control characters and every other C0
# control plus DEL as `\\u{..}` with no leading zeros.
__rmc_dbg_len:
    xor %rax, %rax
    xor %rdx, %rdx
.Ldl_loop:
    cmp %rsi, %rdx
    jae .Ldl_done
    movzbl (%rdi,%rdx), %ecx
    inc %rdx
    cmp $34, %cl
    je .Ldl_two
    cmp $92, %cl
    je .Ldl_two
    cmp $10, %cl
    je .Ldl_two
    cmp $13, %cl
    je .Ldl_two
    cmp $9, %cl
    je .Ldl_two
    cmp $32, %cl
    jb .Ldl_uni
    cmp $127, %cl
    je .Ldl_uni
    inc %rax
    jmp .Ldl_loop
.Ldl_two:
    add $2, %rax
    jmp .Ldl_loop
.Ldl_uni:
    add $5, %rax             # \u{d}
    cmp $16, %ecx
    jb .Ldl_loop
    inc %rax                 # \u{dd}
    jmp .Ldl_loop
.Ldl_done:
    ret
.size __rmc_dbg_len, .-__rmc_dbg_len

# long __rmc_escape_one(char c)  -- the escape sequence for `%cl`, written to
# the 16-byte scratch `%r14`; returns its length. A byte that needs no escaping
# is never passed here.
__rmc_escape_one:
    cmp $34, %cl
    je .Le1_two
    cmp $92, %cl
    je .Le1_two
    cmp $10, %cl
    je .Le1_n
    cmp $13, %cl
    je .Le1_r
    cmp $9, %cl
    je .Le1_t
    movb $92, (%r14)
    movb $117, 1(%r14)       # 'u'
    movb $123, 2(%r14)       # '{'
    lea __rmc_hexdigit(%rip), %rsi
    movzbl %cl, %eax
    cmp $16, %eax
    jb .Le1_one
    mov %eax, %edx
    shr $4, %edx
    movzbl (%rsi,%rdx), %edx
    mov %dl, 3(%r14)
    mov %eax, %edx
    and $15, %edx
    movzbl (%rsi,%rdx), %edx
    mov %dl, 4(%r14)
    movb $125, 5(%r14)       # '}'
    mov $6, %eax
    ret
.Le1_one:
    movzbl (%rsi,%rax), %edx
    mov %dl, 3(%r14)
    movb $125, 4(%r14)
    mov $5, %eax
    ret
.Le1_two:
    movb $92, (%r14)
    mov %cl, 1(%r14)
    mov $2, %eax
    ret
.Le1_n:
    movb $92, (%r14)
    movb $110, 1(%r14)
    mov $2, %eax
    ret
.Le1_r:
    movb $92, (%r14)
    movb $114, 1(%r14)
    mov $2, %eax
    ret
.Le1_t:
    movb $92, (%r14)
    movb $116, 1(%r14)
    mov $2, %eax
    ret
.size __rmc_escape_one, .-__rmc_escape_one

# void __rmc_dbg_body(const char *s, long n)  -- write `"`, the escaped bytes of
# the `n`-byte string `s`, then `"`.
__rmc_dbg_body:
    push %rbx
    push %rbp
    push %r12
    push %r13
    push %r14
    sub $32, %rsp
    mov %rdi, %rbx
    mov %rsi, %rbp
    lea 16(%rsp), %r14       # the escape scratch
    movb $34, (%rsp)
    mov $1, %edx
    mov %rsp, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    xor %r12, %r12           # cursor
    xor %r13, %r13           # start of the pending plain run
.Ldb_loop:
    cmp %rbp, %r12
    jae .Ldb_tail
    movzbl (%rbx,%r12), %ecx
    cmp $34, %cl
    je .Ldb_esc
    cmp $92, %cl
    je .Ldb_esc
    cmp $10, %cl
    je .Ldb_esc
    cmp $13, %cl
    je .Ldb_esc
    cmp $9, %cl
    je .Ldb_esc
    cmp $32, %cl
    jb .Ldb_esc
    cmp $127, %cl
    je .Ldb_esc
    inc %r12
    jmp .Ldb_loop
.Ldb_esc:
    mov %r12, %rdx
    sub %r13, %rdx
    jz .Ldb_noflush
    lea (%rbx,%r13), %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
.Ldb_noflush:
    movzbl (%rbx,%r12), %ecx
    call __rmc_escape_one
    mov %rax, %rdx
    mov %r14, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    inc %r12
    mov %r12, %r13
    jmp .Ldb_loop
.Ldb_tail:
    mov %r12, %rdx
    sub %r13, %rdx
    jz .Ldb_close
    lea (%rbx,%r13), %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
.Ldb_close:
    movb $34, (%rsp)
    mov $1, %edx
    mov %rsp, %rsi
    mov $1, %rdi
    mov $1, %rax
    syscall
    add $32, %rsp
    pop %r14
    pop %r13
    pop %r12
    pop %rbp
    pop %rbx
    ret
.size __rmc_dbg_body, .-__rmc_dbg_body

# void __rmc_fmt_str_debug(const char **slot)  -- `{:?}` for a `&str`. The
# value is quoted and escaped. rustc's `Debug for str` writes the value with
# `write_char` and never calls the formatter's padding, so every option --
# width, fill, alignment and precision -- is ignored.
__rmc_fmt_str_debug:
    mov (%rdi), %rdi
    mov (%rdi), %rsi         # len
    lea 8(%rdi), %rdi        # bytes
    jmp __rmc_dbg_body
.size __rmc_fmt_str_debug, .-__rmc_fmt_str_debug

.section .rodata
__rmc_fmt_true:  .ascii "true"
__rmc_fmt_false: .ascii "false"
__rmc_hexdigit:  .ascii "0123456789abcdef"
"#,
        defines: &[
            "__rmc_fmt_i64", "__rmc_fmt_u64", "__rmc_fmt_i32", "__rmc_fmt_u32",
            "__rmc_fmt_bool", "__rmc_fmt_char", "__rmc_fmt_str", "__rmc_fmt_str_debug",
            "__rmc_fmt_set", "__rmc_fmt_pad", "__rmc_put_fill", "__rmc_fmt_left",
            "__rmc_dbg_len", "__rmc_escape_one", "__rmc_dbg_body",
            "__rmc_emit_signed", "__rmc_emit_unsigned",
        ],
    },
    Piece {
        text: r#"
# The `{:x}`/`{:X}`/`{:o}`/`{:b}` formatters. Rust prints the value as an
# unsigned integer of its own width, so each entry point loads exactly that many
# bytes (zero-extending), which drops the sign bits of a narrower type: `{:x}`
# of `-2i16` is `fffe`, not the 16-digit 64-bit pattern. `%r9` is the base and
# `%r10` 1 for a lowercase alphabet, 2 for uppercase.
.text
.globl __rmc_fmt_hex_8
.type __rmc_fmt_hex_8, @function
__rmc_fmt_hex_8:
    movzbl (%rdi), %eax
    mov $16, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_hex_8, .-__rmc_fmt_hex_8

.globl __rmc_fmt_hex_16
.type __rmc_fmt_hex_16, @function
__rmc_fmt_hex_16:
    movzwl (%rdi), %eax
    mov $16, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_hex_16, .-__rmc_fmt_hex_16

.globl __rmc_fmt_hex_32
.type __rmc_fmt_hex_32, @function
__rmc_fmt_hex_32:
    mov (%rdi), %eax
    mov $16, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_hex_32, .-__rmc_fmt_hex_32

.globl __rmc_fmt_hex_64
.type __rmc_fmt_hex_64, @function
__rmc_fmt_hex_64:
    mov (%rdi), %rax
    mov $16, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_hex_64, .-__rmc_fmt_hex_64

.globl __rmc_fmt_HEX_8
.type __rmc_fmt_HEX_8, @function
__rmc_fmt_HEX_8:
    movzbl (%rdi), %eax
    mov $16, %r9
    mov $2, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_HEX_8, .-__rmc_fmt_HEX_8

.globl __rmc_fmt_HEX_16
.type __rmc_fmt_HEX_16, @function
__rmc_fmt_HEX_16:
    movzwl (%rdi), %eax
    mov $16, %r9
    mov $2, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_HEX_16, .-__rmc_fmt_HEX_16

.globl __rmc_fmt_HEX_32
.type __rmc_fmt_HEX_32, @function
__rmc_fmt_HEX_32:
    mov (%rdi), %eax
    mov $16, %r9
    mov $2, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_HEX_32, .-__rmc_fmt_HEX_32

.globl __rmc_fmt_HEX_64
.type __rmc_fmt_HEX_64, @function
__rmc_fmt_HEX_64:
    mov (%rdi), %rax
    mov $16, %r9
    mov $2, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_HEX_64, .-__rmc_fmt_HEX_64

.globl __rmc_fmt_oct_8
.type __rmc_fmt_oct_8, @function
__rmc_fmt_oct_8:
    movzbl (%rdi), %eax
    mov $8, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_oct_8, .-__rmc_fmt_oct_8

.globl __rmc_fmt_oct_16
.type __rmc_fmt_oct_16, @function
__rmc_fmt_oct_16:
    movzwl (%rdi), %eax
    mov $8, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_oct_16, .-__rmc_fmt_oct_16

.globl __rmc_fmt_oct_32
.type __rmc_fmt_oct_32, @function
__rmc_fmt_oct_32:
    mov (%rdi), %eax
    mov $8, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_oct_32, .-__rmc_fmt_oct_32

.globl __rmc_fmt_oct_64
.type __rmc_fmt_oct_64, @function
__rmc_fmt_oct_64:
    mov (%rdi), %rax
    mov $8, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_oct_64, .-__rmc_fmt_oct_64

.globl __rmc_fmt_bin_8
.type __rmc_fmt_bin_8, @function
__rmc_fmt_bin_8:
    movzbl (%rdi), %eax
    mov $2, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_bin_8, .-__rmc_fmt_bin_8

.globl __rmc_fmt_bin_16
.type __rmc_fmt_bin_16, @function
__rmc_fmt_bin_16:
    movzwl (%rdi), %eax
    mov $2, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_bin_16, .-__rmc_fmt_bin_16

.globl __rmc_fmt_bin_32
.type __rmc_fmt_bin_32, @function
__rmc_fmt_bin_32:
    mov (%rdi), %eax
    mov $2, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_bin_32, .-__rmc_fmt_bin_32

.globl __rmc_fmt_bin_64
.type __rmc_fmt_bin_64, @function
__rmc_fmt_bin_64:
    mov (%rdi), %rax
    mov $2, %r9
    mov $1, %r10
    sub $96, %rsp
    mov %rsp, %rdi
    call __rmc_emit_unsigned
    add $96, %rsp
    ret
.size __rmc_fmt_bin_64, .-__rmc_fmt_bin_64
"#,
        defines: &[
            "__rmc_fmt_hex_8", "__rmc_fmt_hex_16", "__rmc_fmt_hex_32", "__rmc_fmt_hex_64",
            "__rmc_fmt_HEX_8", "__rmc_fmt_HEX_16", "__rmc_fmt_HEX_32", "__rmc_fmt_HEX_64",
            "__rmc_fmt_oct_8", "__rmc_fmt_oct_16", "__rmc_fmt_oct_32", "__rmc_fmt_oct_64",
            "__rmc_fmt_bin_8", "__rmc_fmt_bin_16", "__rmc_fmt_bin_32", "__rmc_fmt_bin_64",
        ],
    },
    Piece {
        text: r#"
# Shared integer printers used by the `core::fmt` formatters. `%rax` holds the
# value; both write it in decimal to stdout. They reuse `print_i64`'s digit
# loop shape so a formatted number and a `print_i64` number cost the same.
.text
__rmc_put_dec_unsigned:
    sub $32, %rsp
    lea 31(%rsp), %rsi
    mov $10, %r9
.Lpdu_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %rsi
    mov %dl, (%rsi)
    test %rax, %rax
    jnz .Lpdu_loop
    lea 31(%rsp), %rdx
    sub %rsi, %rdx
    mov $1, %rdi
    mov $1, %rax
    syscall
    add $32, %rsp
    ret

__rmc_put_dec_signed:
    sub $32, %rsp
    lea 31(%rsp), %rsi
    xor %r8, %r8
    test %rax, %rax
    jns .Lpds_pos
    mov $1, %r8
    neg %rax
.Lpds_pos:
    mov $10, %r9
.Lpds_loop:
    xor %rdx, %rdx
    div %r9
    add $48, %dl
    dec %rsi
    mov %dl, (%rsi)
    test %rax, %rax
    jnz .Lpds_loop
    test %r8, %r8
    jz .Lpds_write
    dec %rsi
    movb $45, (%rsi)
.Lpds_write:
    lea 31(%rsp), %rdx
    sub %rsi, %rdx
    mov $1, %rdi
    mov $1, %rax
    syscall
    add $32, %rsp
    ret
"#,
        defines: &["__rmc_put_dec_signed", "__rmc_put_dec_unsigned"],
    },
    Piece {
        text: r#"
# __rmc_utf8_encode(long codepoint) -> long n  -- write the UTF-8 encoding of
# `codepoint` at `%rdi` and return the number of bytes. Only used to print a
# `char`.
.text
__rmc_utf8_encode:
    mov %edi, %eax           # codepoint
    cmp $0x80, %eax
    jb .Lu8_one
    cmp $0x800, %eax
    jb .Lu8_two
    cmp $0x10000, %eax
    jb .Lu8_three
    # four bytes
    mov %eax, %ecx
    shr $18, %ecx
    or $0xF0, %ecx
    mov %cl, 0(%rsi)
    mov %eax, %ecx
    shr $12, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 1(%rsi)
    mov %eax, %ecx
    shr $6, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 2(%rsi)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 3(%rsi)
    mov $4, %eax
    ret
.Lu8_three:
    mov %eax, %ecx
    shr $12, %ecx
    or $0xE0, %ecx
    mov %cl, 0(%rsi)
    mov %eax, %ecx
    shr $6, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 1(%rsi)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 2(%rsi)
    mov $3, %eax
    ret
.Lu8_two:
    mov %eax, %ecx
    shr $6, %ecx
    or $0xC0, %ecx
    mov %cl, 0(%rsi)
    mov %eax, %ecx
    and $0x3F, %ecx
    or $0x80, %ecx
    mov %cl, 1(%rsi)
    mov $2, %eax
    ret
.Lu8_one:
    mov %al, 0(%rsi)
    mov $1, %eax
    ret
"#,
        defines: &["__rmc_utf8_encode"],
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
# long __rmc_imin(long a, long b) / __rmc_imax  -- signed
# long __rmc_umin(long a, long b) / __rmc_umax  -- unsigned
# `a.min(b)` and `a.max(b)` on an integer lower to `<T as Ord>::min`/`max`,
# which the back end rewrites to one of these. A branch rather than `cmov` keeps
# them encodable by the in-process assembler.
.globl __rmc_imin
.type __rmc_imin, @function
__rmc_imin:
    mov %rdi, %rax
    cmp %rsi, %rax
    jle .Limin_done
    mov %rsi, %rax
.Limin_done:
    ret
.size __rmc_imin, .-__rmc_imin

.globl __rmc_imax
.type __rmc_imax, @function
__rmc_imax:
    mov %rdi, %rax
    cmp %rsi, %rax
    jge .Limax_done
    mov %rsi, %rax
.Limax_done:
    ret
.size __rmc_imax, .-__rmc_imax

.globl __rmc_umin
.type __rmc_umin, @function
__rmc_umin:
    mov %rdi, %rax
    cmp %rsi, %rax
    jbe .Lumin_done
    mov %rsi, %rax
.Lumin_done:
    ret
.size __rmc_umin, .-__rmc_umin

.globl __rmc_umax
.type __rmc_umax, @function
__rmc_umax:
    mov %rdi, %rax
    cmp %rsi, %rax
    jae .Lumax_done
    mov %rsi, %rax
.Lumax_done:
    ret
.size __rmc_umax, .-__rmc_umax
"#,
        defines: &["__rmc_imin", "__rmc_imax", "__rmc_umin", "__rmc_umax"],
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

/// The subset of the runtime that `user_asm` reaches.
///
/// `_start` is always kept — it is the process entry — and every piece it pulls
/// in follows. A piece is kept when the program names one of its symbols, and
/// keeping a piece can name more symbols, so the selection runs to a fixpoint.
/// A program that never allocates does not carry the allocator, and one that
/// never indexes an array does not carry the bounds trap.
pub fn for_program(user_asm: &str) -> String {
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

/// The whole freestanding runtime as one assembly translation unit, `_start`
/// included. This is what lets a *differential* test build the rustc reference
/// against the very same runtime rmc links: compile the rustc output to an
/// object and link it with this text and nothing else, and the two binaries
/// share one ABI, one entry point and one syscall layer.
pub fn support_asm() -> String {
    PIECES.iter().map(|p| p.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_asm_is_the_whole_runtime() {
        let s = support_asm();
        assert!(s.contains("_start:"), "the reference links against this entry");
        assert!(s.contains("print_i64:"), "the printers are exported");
        assert!(s.contains("arg_i64:"), "so is the argv reader");
    }

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
    fn every_piece_is_reachable_from_some_symbol() {
        // A piece no program could ever pull in would be dead weight in the
        // table; make sure each one defines at least one symbol.
        for p in PIECES {
            assert!(!p.defines.is_empty(), "a piece with no symbols");
        }
    }
}
