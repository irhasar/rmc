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
static const char *out_path;   // NULL: pick a private file at exit
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
    // Nothing is opened here: an interrupted run (Ctrl+C and friends never
    // reach the destructor) must not truncate the previous capture. The
    // destination is only touched by sampler_fini.
    const char *env = getenv("SAMPLER_OUT");
    if (env && *env) {
        out_path = strdup(env);
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

// A capture can be appended to by a different binary, so a path (or an "exe"
// tag) is not enough to know whether the symbol table the symbolizer was given
// belongs to the run that produced a sample. Identify the executable by its
// content instead: size plus CRC-32, which survives copying to an archive.
static unsigned long crc_table[256];
static void crc32_init(void) {
    for (unsigned long i = 0; i < 256; i++) {
        unsigned long c = i;
        for (int k = 0; k < 8; k++) {
            c = (c & 1) ? 0xEDB88320UL ^ (c >> 1) : (c >> 1);
        }
        crc_table[i] = c;
    }
}

// Returns 0 when the file cannot be read; the symbolizer then falls back to
// comparing paths.
static int file_id(const char *path, unsigned long long *size, unsigned long *crc) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        return 0;
    }
    crc32_init();
    unsigned long c = 0xFFFFFFFFUL;
    unsigned long long total = 0;
    static unsigned char buf[65536];
    ssize_t n;
    while ((n = read(fd, buf, sizeof(buf))) > 0) {
        for (ssize_t i = 0; i < n; i++) {
            c = crc_table[(c ^ buf[i]) & 0xFF] ^ (c >> 8);
        }
        total += (unsigned long long)n;
    }
    close(fd);
    if (n < 0) {
        return 0;
    }
    *size = total;
    *crc = (c ^ 0xFFFFFFFFUL) & 0xFFFFFFFFUL;
    return 1;
}

// Record the executable mappings so sym.py can tell executable samples from
// library ones instead of counting the latter as "outside any symbol". Each
// mapping carries a tag: "exe" for the executable itself, "lib" for everything
// else. The tag is what lets a profile be symbolized against a copy of the
// binary (an archived run), where the captured path no longer exists.
static void dump_maps(FILE *f) {
    FILE *m = fopen("/proc/self/maps", "r");
    if (!m) {
        return;
    }
    char exe[4096];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
    exe[n > 0 ? n : 0] = 0;
    unsigned long long exe_size = 0;
    unsigned long exe_crc = 0;
    if (file_id(exe, &exe_size, &exe_crc)) {
        fprintf(f, "EXEID %llu %lx\n", exe_size, exe_crc);
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
        fprintf(f, "MAP %llx %llx %s %s\n", start, end, strcmp(path, exe) == 0 ? "exe" : "lib", path);
    }
    fclose(m);
}

static void write_profile(FILE *f) {
    fprintf(f, "BASE %llx\n", load_base);
    dump_maps(f);
    unsigned long n = n_samples < MAX_SAMPLES ? n_samples : MAX_SAMPLES;
    for (unsigned long i = 0; i < n; i++) {
        fprintf(f, "%llx\n", samples[i]);
    }
}

// Returns 1 only when every byte reached the file. A failed buffered write sets
// the stream's error indicator and can still let `fclose` succeed (there is
// nothing left to flush), so both have to be checked before the temporary file
// is allowed to replace a good capture.
static int write_profile_fd(int fd) {
    FILE *f = fdopen(fd, "w");
    if (!f) {
        close(fd);
        return 0;
    }
    write_profile(f);
    int write_failed = ferror(f) != 0;
    int close_failed = fclose(f) != 0;
    return !write_failed && !close_failed;
}

// Overwrite mode writes a private file next to the destination and renames it
// into place, so an interrupted or failed capture leaves the previous profile
// intact. `rename` replaces a symlink rather than following it, which is why
// the destination itself never needs to be opened.
static void write_profile_atomic(const char *path) {
    char tmp[4096];
    const char *slash = strrchr(path, '/');
    if (slash) {
        int dir_len = (int)(slash - path);
        if (dir_len == 0) {
            dir_len = 1;
        }
        snprintf(tmp, sizeof(tmp), "%.*s/.sampler.XXXXXX", dir_len, path);
    } else {
        snprintf(tmp, sizeof(tmp), ".sampler.XXXXXX");
    }
    int fd = mkstemp(tmp);
    if (fd < 0) {
        fprintf(stderr, "sampler: cannot create %s: %s\n", tmp, strerror(errno));
        return;
    }
    fchmod(fd, 0600);
    if (!write_profile_fd(fd)) {
        unlink(tmp);
        fprintf(stderr, "sampler: write to %s failed, keeping the previous capture\n", path);
        return;
    }
    if (rename(tmp, path) != 0) {
        fprintf(stderr, "sampler: cannot replace %s: %s\n", path, strerror(errno));
        unlink(tmp);
    }
}

__attribute__((destructor)) static void sampler_fini(void) {
    struct itimerval it;
    memset(&it, 0, sizeof(it));
    setitimer(ITIMER_PROF, &it, NULL);
    if (!samples) {
        return;
    }
    if (!out_path) {
        // Default: a fresh private file, so neither an existing file nor a
        // symlink in the temp directory can be reused (CWE-377).
        const char *dir = getenv("TMPDIR");
        if (!dir || !*dir) {
            dir = "/tmp";
        }
        char path[4096];
        snprintf(path, sizeof(path), "%s/sampler.XXXXXX", dir);
        int fd = mkstemp(path);
        if (fd < 0) {
            fprintf(stderr, "sampler: cannot create a profile file in %s\n", dir);
            return;
        }
        fchmod(fd, 0600);
        if (!write_profile_fd(fd)) {
            unlink(path);
            fprintf(stderr, "sampler: write to %s failed\n", path);
            return;
        }
        fprintf(stderr, "sampler: wrote %s\n", path);
        return;
    }
    if (getenv("SAMPLER_APPEND")) {
        // Appending cannot destroy what is already there, even if the run is
        // interrupted halfway through the write.
        int fd = open(out_path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC | O_NOFOLLOW, 0600);
        if (fd < 0) {
            fprintf(stderr, "sampler: cannot open %s: %s\n", out_path, strerror(errno));
            return;
        }
        // Appending cannot be undone, but a failed write must at least be
        // reported rather than leaving a silently truncated tail.
        if (!write_profile_fd(fd)) {
            fprintf(stderr, "sampler: write to %s failed: %s\n", out_path, strerror(errno));
        }
        return;
    }
    write_profile_atomic(out_path);
}
