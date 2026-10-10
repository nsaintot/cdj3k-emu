// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * clock.c - a clock_gettime and gettimeofday hook that shifts the time
 * seen by the OptFstUdpServer thread only.
 *
 * In slave mode, OptFstUdpServer reads the master's broadcast position and
 * uses its own clock to project where the master is now. The hook subtracts
 * N ms from the time this thread reads, which moves this deck's playback by
 * N ms to compensate for the latency of the host audio pipeline (QEMU, the
 * host audio API and the output device, typically 50 to 100 ms). The audible
 * playback then lines up with the master's.
 *
 * The shifted thread reads these sysfs parameters once per wall-clock second:
 *   audio_sync_enabled   The master switch. At 0, this hook and the
 *                        master-mode delayed send in link.c do nothing; at
 *                        1, both are active.
 *   audio_latency_ms     The live pipeline depth, read-only, and the default
 *                        source. It follows the actual pipeline, so the
 *                        slave's audible output stays aligned with the
 *                        master's in steady state. The kernel caps it at
 *                        200 ms; when the backlog persists, a watchdog
 *                        forces an xrun to recover.
 *   link_pos_offset_ms   The manual override; a positive value forces a
 *                        fixed shift.
 *
 * This hook does not change what a master broadcasts; link.c compensates
 * master mode by delaying the Pro DJ Link broadcasts.
 *
 * Setting DECK_TIME_SHIFT_DEBUG logs the first shift in each thread and
 * every change of the offset.
 */

#include "deck_shim.h"
#include <elf.h>
#include <sys/auxv.h>
#include <sys/prctl.h>
#include <sys/time.h>

#ifndef PR_GET_NAME
#define PR_GET_NAME 16
#endif

#define TARGET_THREAD    "OptFstUdpServer"

static __thread int          g_thread_shift = -1;   /* -1 unset, 0 passthrough, 1 shift */
static __thread char         g_thread_name[16] = {0};
static __thread int          g_thread_did_log_apply = 0;
static __thread long         g_thread_last_refresh_s = 0;

static long g_shift_ns_atomic = 0;
static long g_last_logged_ms = -1;
static long g_last_logged_enabled = -1;
static int  g_shift_debug;

static void __attribute__((constructor)) shim_clock_init(void)
{
    g_shift_debug = (getenv("DECK_TIME_SHIFT_DEBUG") != NULL);
}

static void refresh_shift_ms(void)
{
    /* When audio_sync_enabled is 0, the clock shift is off, whatever
     * audio_latency_ms and link_pos_offset_ms say. The same switch controls
     * the master-mode delayed send in link.c, so it turns all audio-sync
     * compensation in the shim on or off together. */
    long enabled = shim_read_sysfs_long(VSND_LATENCY_ENABLED_PATH);
    if (enabled <= 0) {
        long prev = __atomic_load_n(&g_shift_ns_atomic, __ATOMIC_RELAXED);
        if (prev != 0)
            __atomic_store_n(&g_shift_ns_atomic, 0, __ATOMIC_RELAXED);
        if (g_shift_debug && g_last_logged_enabled != 0) {
            g_last_logged_enabled = 0;
            fprintf(stderr, "[deck_clock] The clock shift is off (audio_sync_enabled=0).\n");
            fflush(stderr);
        }
        return;
    }
    if (g_shift_debug && g_last_logged_enabled != 1) {
        g_last_logged_enabled = 1;
        fprintf(stderr, "[deck_clock] The clock shift is on.\n");
        fflush(stderr);
    }

    /* A positive link_pos_offset_ms overrides the automatic value; otherwise
     * the shift follows audio_latency_ms, the live pipeline depth. */
    long manual = shim_read_sysfs_long(VSND_LATENCY_MANUAL_PATH);
    long ms;
    if (manual > 0) {
        ms = manual;
    } else {
        ms = shim_read_sysfs_long(VSND_LATENCY_AUTO_PATH);
        if (ms < 0) ms = 0;
    }
    if (ms > 5000) ms = 5000;
    long next = ms * 1000000L;
    long prev = __atomic_load_n(&g_shift_ns_atomic, __ATOMIC_RELAXED);
    if (prev == next) return;
    __atomic_store_n(&g_shift_ns_atomic, next, __ATOMIC_RELAXED);
    if (g_shift_debug && g_last_logged_ms != ms) {
        g_last_logged_ms = ms;
        fprintf(stderr, "[deck_clock] The shift is now %ld ms (%s).\n",
                ms, manual > 0 ? "manual" : "auto");
        fflush(stderr);
    }
}

/* Returns 1 if this thread is OptFstUdpServer, else 0. The name is cached for
 * the life of the thread, because it does not change after the first prctl. */
static int classify_self_thread(void)
{
    if (g_thread_name[0] == '\0') {
        if (syscall(SYS_prctl, PR_GET_NAME,
                    (unsigned long)g_thread_name, 0, 0, 0) != 0) {
            g_thread_name[0] = '?';
            g_thread_name[1] = '\0';
            return 0;
        }
        g_thread_name[15] = '\0';
    }
    int match = (strcmp(g_thread_name, TARGET_THREAD) == 0);
    if (g_shift_debug && match) {
        fprintf(stderr, "[deck_clock] Shifting the clock of thread '%s'.\n", g_thread_name);
        fflush(stderr);
    }
    return match;
}

/* Find a function that the kernel's vDSO exports, through its ELF dynamic
 * symbol table. The arm64 vDSO links with a SysV hash, whose nchain is the
 * symbol count. Returns NULL when the vDSO or the symbol is absent. */
static void *vdso_sym(const char *name)
{
    unsigned long base = getauxval(AT_SYSINFO_EHDR);
    if (!base) return NULL;
    const Elf64_Ehdr *eh = (const Elf64_Ehdr *)base;
    const unsigned char *ph = (const unsigned char *)base + eh->e_phoff;
    unsigned long load = 0;
    int have_load = 0;
    const Elf64_Dyn *dyn = NULL;
    for (int i = 0; i < eh->e_phnum; i++, ph += eh->e_phentsize) {
        const Elf64_Phdr *p = (const Elf64_Phdr *)ph;
        if (p->p_type == PT_LOAD && !have_load) {
            load = base + p->p_offset - p->p_vaddr;
            have_load = 1;
        } else if (p->p_type == PT_DYNAMIC) {
            dyn = (const Elf64_Dyn *)(base + p->p_offset);
        }
    }
    if (!have_load || !dyn) return NULL;
    const char *strtab = NULL;
    const Elf64_Sym *symtab = NULL;
    const Elf32_Word *hash = NULL;
    for (; dyn->d_tag != DT_NULL; dyn++) {
        switch (dyn->d_tag) {
        case DT_STRTAB: strtab = (const char *)(load + dyn->d_un.d_ptr); break;
        case DT_SYMTAB: symtab = (const Elf64_Sym *)(load + dyn->d_un.d_ptr); break;
        case DT_HASH:   hash = (const Elf32_Word *)(load + dyn->d_un.d_ptr); break;
        }
    }
    if (!strtab || !symtab || !hash) return NULL;
    for (Elf32_Word i = 0; i < hash[1]; i++) {
        const Elf64_Sym *sym = &symtab[i];
        if (ELF64_ST_TYPE(sym->st_info) != STT_FUNC || sym->st_shndx == SHN_UNDEF)
            continue;
        if (strcmp(strtab + sym->st_name, name) == 0)
            return (void *)(load + sym->st_value);
    }
    return NULL;
}

typedef int (*vdso_clock_gettime_fn)(clockid_t, struct timespec *);
typedef int (*vdso_gettimeofday_fn)(struct timeval *, void *);

static vdso_clock_gettime_fn g_vdso_clock_gettime;
static vdso_gettimeofday_fn  g_vdso_gettimeofday;

/* Resolve the vDSO entry points, so that a passthrough reads the clock in
 * user space; a raw syscall would make every clock read of every thread a
 * trap. */
static void __attribute__((constructor)) shim_vdso_init(void)
{
    g_vdso_clock_gettime =
        (vdso_clock_gettime_fn)vdso_sym("__kernel_clock_gettime");
    g_vdso_gettimeofday =
        (vdso_gettimeofday_fn)vdso_sym("__kernel_gettimeofday");
}

/* The vDSO returns 0 or a negative errno. */
static int real_clock_gettime(clockid_t clk_id, struct timespec *tp)
{
    if (g_vdso_clock_gettime) {
        int r = g_vdso_clock_gettime(clk_id, tp);
        if (r < 0) { errno = -r; return -1; }
        return 0;
    }
    long r = syscall(SYS_clock_gettime, clk_id, tp);
    if (r < 0) return -1;   /* glibc syscall() already set errno */
    return 0;
}

/* Subtract ns from the reported time; this is the direction that aligns
 * slave-mode sync through OptFstUdpServer. */
static void apply_shift_ts(struct timespec *tp)
{
    long ns = __atomic_load_n(&g_shift_ns_atomic, __ATOMIC_RELAXED);
    if (ns == 0) return;
    long before_sec = tp->tv_sec, before_ns = tp->tv_nsec;
    if (tp->tv_sec > 0 || tp->tv_nsec >= ns) {
        if (tp->tv_nsec >= ns) tp->tv_nsec -= ns;
        else { tp->tv_nsec += 1000000000L - ns; tp->tv_sec--; }
    }
    if (g_shift_debug && !g_thread_did_log_apply) {
        g_thread_did_log_apply = 1;
        fprintf(stderr,
                "[deck_clock] '%s' apply_shift: shifted %ld.%09ld to %ld.%09ld (ns=%ld)\n",
                g_thread_name, before_sec, before_ns,
                (long)tp->tv_sec, (long)tp->tv_nsec, ns);
        fflush(stderr);
    }
}

static void apply_shift_tv(struct timeval *tv)
{
    long ns = __atomic_load_n(&g_shift_ns_atomic, __ATOMIC_RELAXED);
    long us = ns / 1000;
    if (us == 0) return;
    if (tv->tv_sec > 0 || tv->tv_usec >= us) {
        if (tv->tv_usec >= us) tv->tv_usec -= us;
        else { tv->tv_usec += 1000000L - us; tv->tv_sec--; }
    }
}

int clock_gettime(clockid_t clk_id, struct timespec *tp)
{
    int rc = real_clock_gettime(clk_id, tp);
    if (rc != 0) return rc;

    if (clk_id != CLOCK_MONOTONIC && clk_id != CLOCK_MONOTONIC_RAW
     && clk_id != CLOCK_REALTIME) {
        return 0;
    }

    if (g_thread_shift < 0)
        g_thread_shift = classify_self_thread();

    /* Only the shifted thread reads sysfs, so the JUCE ALSA thread and the
     * other audio threads stay on the fast path. The refresh runs at most
     * once per wall-clock second and uses the timestamp just read, so it
     * costs no extra syscall. */
    if (g_thread_shift == 0) return 0;

    if ((long)tp->tv_sec - g_thread_last_refresh_s >= VSND_LATENCY_REFRESH_S) {
        g_thread_last_refresh_s = (long)tp->tv_sec;
        refresh_shift_ms();
    }

    apply_shift_ts(tp);
    return 0;
}

#ifdef __GLIBC__
int gettimeofday(struct timeval *tv, struct timezone *tz)
#else
int gettimeofday(struct timeval *tv, void *tz)
#endif
{
    if (g_vdso_gettimeofday) {
        int r = g_vdso_gettimeofday(tv, (void *)tz);
        if (r < 0) { errno = -r; return -1; }
    } else {
        long r = syscall(SYS_gettimeofday, tv, tz);
        if (r < 0) return -1;   /* glibc syscall() already set errno */
    }
    if (!tv) return 0;

    if (g_thread_shift < 0)
        g_thread_shift = classify_self_thread();

    if (g_thread_shift == 0) return 0;

    if ((long)tv->tv_sec - g_thread_last_refresh_s >= VSND_LATENCY_REFRESH_S) {
        g_thread_last_refresh_s = (long)tv->tv_sec;
        refresh_shift_ms();
    }

    apply_shift_tv(tv);
    return 0;
}
