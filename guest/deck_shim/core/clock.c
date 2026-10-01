// SPDX-License-Identifier: MIT OR Apache-2.0
/*
 * clock.c - clock_gettime / gettimeofday hook that shifts
 * the wall-clock view for the OptFstUdpServer thread only.
 *
 * Effect: in slave mode, OptFstUdpServer reads master's broadcast
 * position and projects "where master is now" using its own clock.
 * Shifting its clock forward by N ms makes it think it's N ms further
 * along, so it adjusts our playback to land N ms earlier - which
 * compensates the host audio pipeline latency (QEMU + CoreAudio +
 * Focusrite, ~50–100 ms typical) so audible playback aligns with
 * master's audible playback.
 *
 * Sysfs surfaces (read once per wall-clock second by the shifted thread):
 *   audio_sync_enabled   (master switch; 0 = full no-op for this hook
 *                         and link.c, 1 = both active. Same toggle gates
 *                         the master-mode delay-send in link.c.)
 *   audio_latency_ms     (read-only, live pipeline depth - default source.
 *                         Tracks actual pipeline so slave audible aligns with
 *                         master audible in steady state. Capped at 200ms in
 *                         the kernel; deep-state recovery via the watchdog
 *                         that forces an xrun on sustained backlog.)
 *   link_pos_offset_ms   (manual override; non-zero forces fixed value)
 *
 * Master mode is unaffected - its broadcast threads don't go through
 * libc clock_gettime in a way the shim can intercept; broadcast
 * position remains aligned to JUCE's pre-pipeline view.
 *
 * Diagnostic mode (env DECK_TIME_SHIFT_DEBUG=1) logs once per thread
 * the first time the shift is applied, plus offset value changes.
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

static __thread int          g_thread_shift = -1;   /* -1=unset, 0=passthrough, 1=shift */
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
    /* Master switch - when audio_sync_enabled is 0, the shim is fully
     * inert (no clock shift, regardless of audio_latency_ms or
     * link_pos_offset_ms values). Same toggle gates the master-mode
     * delay-send shim, so flipping it produces a coherent on/off for
     * the entire LD_PRELOAD audio-sync compensation pipeline. */
    long enabled = shim_read_sysfs_long(VSND_LATENCY_ENABLED_PATH);
    if (enabled <= 0) {
        long prev = __atomic_load_n(&g_shift_ns_atomic, __ATOMIC_RELAXED);
        if (prev != 0)
            __atomic_store_n(&g_shift_ns_atomic, 0, __ATOMIC_RELAXED);
        if (g_shift_debug && g_last_logged_enabled != 0) {
            g_last_logged_enabled = 0;
            fprintf(stderr, "[deck_clock] disabled (audio_sync_enabled=0)\n");
            fflush(stderr);
        }
        return;
    }
    if (g_shift_debug && g_last_logged_enabled != 1) {
        g_last_logged_enabled = 1;
        fprintf(stderr, "[deck_clock] enabled\n");
        fflush(stderr);
    }

    /* Manual override wins: if link_pos_offset_ms is non-zero, use it.
     * Otherwise auto-track audio_latency_ms (live pipeline depth). */
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
        fprintf(stderr, "[deck_clock] shift = %ld ms (%s)\n",
                ms, manual > 0 ? "manual" : "auto");
        fflush(stderr);
    }
}

/* Returns 1 if this thread is OptFstUdpServer, else 0. Cached per-thread
 * for life - thread name doesn't change after first prctl. */
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
        fprintf(stderr, "[deck_clock] thread '%s' → SHIFT\n", g_thread_name);
        fflush(stderr);
    }
    return match;
}

/* A function exported by the kernel's vDSO, found through its ELF dynamic
 * symbol table (the arm64 vDSO links with a SysV hash, whose nchain is the
 * symbol count). NULL when the vDSO or the symbol is absent. */
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

/* The vDSO entry points, so a passthrough reads the clock in user space; a
 * raw syscall here would make every clock read of every thread a trap. */
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

/* Subtract ns from the reported time. (Empirically the direction that
 * makes slave-mode sync align via OptFstUdpServer.) */
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
                "[deck_clock] '%s' apply_shift: %ld.%09ld -> %ld.%09ld (ns=%ld)\n",
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

    /* Only the shifted thread pays for sysfs reads - keeps JUCE ALSA and
     * other audio-critical threads on a hot path. Refresh at most once
     * per wall-clock second using the timestamp we just got from the
     * kernel (no extra syscall). */
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
