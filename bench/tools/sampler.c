// A sampling profiler for binaries that cannot be run under `perf`.
//
// LD_PRELOAD this into the target: the constructor arms ITIMER_PROF, the
// SIGPROF handler records the interrupted instruction pointer, and the
// destructor writes "<count> <addr>" lines to $SAMPLER_OUT (default
// /tmp/sampler.out). Symbolise with sym.py.
//
// A signal-based sampler only sees running user code, which is exactly what a
// tight benchmark loop needs: libc time shows up as the libc frame.
#define _GNU_SOURCE
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <ucontext.h>
#include <errno.h>
#include <unistd.h>

#define MAX_SAMPLES (1 << 22)

static unsigned long long *samples;
static volatile unsigned long n_samples;
static int out_fd = -1;
static unsigned long long load_base;

// The target is a PIE, so the sampled instruction pointers are runtime
// addresses. Record the executable's load base once and subtract it, so the
// samples can be looked up in the on-disk symbol table.
static unsigned long long executable_base(void) {
    FILE *f = fopen("/proc/self/maps", "r");
    if (!f) {
        return 0;
    }
    char line[4096];
    char exe[4096];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
    if (n <= 0) {
        fclose(f);
        return 0;
    }
    exe[n] = 0;
    unsigned long long base = 0;
    while (fgets(line, sizeof(line), f)) {
        unsigned long long start;
        char perms[8], path[4096];
        path[0] = 0;
        if (sscanf(line, "%llx-%*llx %7s %*s %*s %*s %4095s", &start, perms, path) < 2) {
            continue;
        }
        if (path[0] && strcmp(path, exe) == 0) {
            base = start;
            break;
        }
    }
    fclose(f);
    return base;
}

static void on_prof(int sig, siginfo_t *info, void *uctx) {
    (void)sig;
    (void)info;
    ucontext_t *uc = (ucontext_t *)uctx;
    unsigned long long ip = (unsigned long long)uc->uc_mcontext.gregs[REG_RIP];
    unsigned long i = __atomic_fetch_add(&n_samples, 1, __ATOMIC_RELAXED);
    if (i < MAX_SAMPLES) {
        samples[i] = ip;
    }
}

__attribute__((constructor)) static void sampler_init(void) {
    samples = calloc(MAX_SAMPLES, sizeof(*samples));
    if (!samples) {
        return;
    }
    const char *env = getenv("SAMPLER_OUT");
    if (env) {
        // An explicit path is the caller's choice, but never follow a symlink
        // (a planted link would redirect the write to a file the caller owns)
        // and keep the result private.
        int flags = O_WRONLY | O_CREAT | O_CLOEXEC | O_NOFOLLOW;
        flags |= getenv("SAMPLER_APPEND") ? O_APPEND : O_TRUNC;
        out_fd = open(env, flags, 0600);
        if (out_fd < 0) {
            fprintf(stderr, "sampler: cannot open %s: %s\n", env, strerror(errno));
        }
    } else {
        // Default: create a fresh private file with O_EXCL, so neither an
        // existing file nor a symlink in the temp directory can be reused
        // (CWE-377).
        const char *dir = getenv("TMPDIR");
        if (!dir || !*dir) {
            dir = "/tmp";
        }
        char path[4096];
        snprintf(path, sizeof(path), "%s/sampler.XXXXXX", dir);
        out_fd = mkstemp(path);
        if (out_fd >= 0) {
            fchmod(out_fd, 0600);
            fprintf(stderr, "sampler: writing %s\n", path);
        } else {
            fprintf(stderr, "sampler: cannot create a profile file in %s\n", dir);
        }
    }
    load_base = executable_base();
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = on_prof;
    sa.sa_flags = SA_SIGINFO | SA_RESTART;
    sigaction(SIGPROF, &sa, NULL);
    struct itimerval it;
    it.it_interval.tv_sec = 0;
    it.it_interval.tv_usec = 1000; // 1 kHz
    it.it_value = it.it_interval;
    setitimer(ITIMER_PROF, &it, NULL);
}

// Record the executable mappings so sym.py can tell executable samples from
// library ones instead of counting the latter as "outside any symbol".
static void dump_maps(FILE *f) {
    FILE *m = fopen("/proc/self/maps", "r");
    if (!m) {
        return;
    }
    char line[4096];
    while (fgets(line, sizeof(line), m)) {
        unsigned long long start, end;
        char perms[8], path[4096];
        path[0] = 0;
        if (sscanf(line, "%llx-%llx %7s %*s %*s %*s %4095s", &start, &end, perms, path) < 3) {
            continue;
        }
        if (path[0] != '/' || !strchr(perms, 'x')) {
            continue;
        }
        fprintf(f, "MAP %llx %llx %s\n", start, end, path);
    }
    fclose(m);
}

__attribute__((destructor)) static void sampler_fini(void) {
    struct itimerval it;
    memset(&it, 0, sizeof(it));
    setitimer(ITIMER_PROF, &it, NULL);
    if (out_fd < 0) {
        return;
    }
    FILE *f = fdopen(out_fd, "w");
    if (!f) {
        close(out_fd);
        return;
    }
    fprintf(f, "BASE %llx\n", load_base);
    dump_maps(f);
    unsigned long n = n_samples < MAX_SAMPLES ? n_samples : MAX_SAMPLES;
    for (unsigned long i = 0; i < n; i++) {
        fprintf(f, "%llx\n", samples[i]);
    }
    fclose(f);
}
