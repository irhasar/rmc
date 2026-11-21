// Minimal userspace sampling profiler: forks the target, then repeatedly
// continues it for one interval and interrupts it with SIGSTOP, recording RIP.
// Needs no privileges beyond ptrace on its own child, so it works where `perf`
// is unavailable. Emits one RIP per line to stdout.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <sys/user.h>
#include <time.h>

#define MAX_SAMPLES 400000

static unsigned long rips[MAX_SAMPLES];
static int n_samples;

static void sleep_us(long us) {
    struct timespec ts = { us / 1000000, (us % 1000000) * 1000 };
    nanosleep(&ts, NULL);
}

static unsigned long g_lo, g_hi, g_bias;

// A PIE executable is mapped at a randomised base, so raw RIPs cannot be looked
// up in the symbol table directly. The load bias cannot be derived from the
// mapping start and its file offset: the executable PT_LOAD has p_offset
// 0x1ef90 but p_vaddr 0x1ff90 in these binaries, so those differ by a page.
// Read p_vaddr from the ELF header and use `start - page_align(p_vaddr)`.
static void find_range(pid_t pid, const char *prog) {
    unsigned long vaddr_page = 0, off_page = 0;
    FILE *elf = fopen(prog, "rb");
    if (elf) {
        unsigned char eh[64];
        if (fread(eh, 1, sizeof eh, elf) == sizeof eh && eh[0] == 0x7f) {
            unsigned long phoff = *(unsigned long *)(eh + 0x20);
            unsigned short phentsize = *(unsigned short *)(eh + 0x36);
            unsigned short phnum = *(unsigned short *)(eh + 0x38);
            for (unsigned i = 0; i < phnum; i++) {
                unsigned char ph[56];
                fseek(elf, phoff + (unsigned long)i * phentsize, SEEK_SET);
                if (fread(ph, 1, sizeof ph, elf) != sizeof ph) break;
                unsigned type = *(unsigned *)(ph + 0x00);
                unsigned flags = *(unsigned *)(ph + 0x04);
                unsigned long p_off = *(unsigned long *)(ph + 0x08);
                unsigned long p_va = *(unsigned long *)(ph + 0x10);
                if (type == 1 /* PT_LOAD */ && (flags & 1) /* PF_X */) {
                    vaddr_page = p_va & ~0xfffUL;
                    off_page = p_off & ~0xfffUL;
                    break;
                }
            }
        }
        fclose(elf);
    }

    char path[64], line[512];
    snprintf(path, sizeof path, "/proc/%d/maps", pid);
    FILE *f = fopen(path, "r");
    if (!f) return;
    const char *slash = strrchr(prog, '/');
    const char *want = slash ? slash + 1 : prog;
    while (fgets(line, sizeof line, f)) {
        if (!strstr(line, want)) continue;
        if (!strstr(line, "r-xp")) continue;
        unsigned long start = 0, end = 0, off = 0;
        if (sscanf(line, "%lx-%lx %*4s %lx", &start, &end, &off) == 3) {
            // Match the mapping to the executable PT_LOAD by file offset.
            if (off_page && (off & ~0xfffUL) != off_page) continue;
            g_lo = start;
            g_hi = end;
            g_bias = start - vaddr_page;
        }
        break;
    }
    fclose(f);
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <interval_us> <prog> [args...]\n", argv[0]);
        return 2;
    }
    long interval = atol(argv[1]);

    pid_t pid = fork();
    if (pid == 0) {
        ptrace(PTRACE_TRACEME, 0, 0, 0);
        execv(argv[2], &argv[2]);
        perror("execv");
        _exit(127);
    }

    int status;
    if (waitpid(pid, &status, 0) < 0) { perror("waitpid"); return 1; }

    unsigned long base = 0;
    while (n_samples < MAX_SAMPLES) {
        if (ptrace(PTRACE_CONT, pid, 0, 0) < 0) break;
        sleep_us(interval);
        kill(pid, SIGSTOP);
        if (waitpid(pid, &status, 0) < 0) break;
        if (WIFEXITED(status) || WIFSIGNALED(status)) break;

        struct user_regs_struct regs;
        if (ptrace(PTRACE_GETREGS, pid, 0, &regs) < 0) break;
        if (!base) { find_range(pid, argv[2]); base = g_bias; }
        rips[n_samples++] = regs.rip;
    }

    if (WIFSTOPPED(status)) {
        ptrace(PTRACE_CONT, pid, 0, 0);
        kill(pid, SIGKILL);
        waitpid(pid, &status, 0);
    }

    fprintf(stderr, "lo=%lx hi=%lx bias=%lx samples=%d\n",
            g_lo, g_hi, base, n_samples);
    for (int i = 0; i < n_samples; i++) {
        // Samples in libc are not part of the target; mark them so the
        // attribution step can report them separately instead of aliasing.
        if (rips[i] >= g_lo && rips[i] < g_hi)
            printf("%lx\n", rips[i] - base);
        else
            printf("ext\n");
    }
    return 0;
}
