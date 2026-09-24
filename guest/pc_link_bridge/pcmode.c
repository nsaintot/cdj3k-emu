// SPDX-License-Identifier: MIT OR Apache-2.0
/* pcmode.c: force EP122 PC mode.
 *
 * The deck gates every HID/MIDI report to the host behind
 * connection_with_hostapp::PcModeSwitcher: the HID send path and
 * SendManager::midiSend only run once PC mode is active, and it is normally
 * set by a host-driven cert handshake that can't be reproduced against
 * our virtual device. The getter is `mutex_lock; w0 = *(this+8); mutex_unlock; return w0`;
 * its load of the flag becomes `mov w20,#1`, so it reports PC mode on.
 *
 * Only EP122's main thread is stopped while the text is poked.  One
 * instruction changes and it does not touch the stack, so a thread that is
 * inside the getter when the page flips runs either the old or the new word
 * with the same frame.
 *
 * It is located by matching the instruction words of its body, with the two
 * PC-relative `bl` masked.  rk3399 builds 3.13 to 3.22 each match once.
 * See docs/pc-link.md. */

#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "bridge.h"

/* The flag load, at getter + 28, and what replaces it. */
#define PCMODE_LOAD_OFF     28
#define EP122_PCMODE_ORIG   0xb9400a94u /* ldr w20, [x20, #8] */
#define EP122_PCMODE_PATCH  0x52800034u /* mov w20, #1        */

// PTRACE_POKETEXT writes 8 bytes from the aligned doubleword containing the target word; this avoids partial updates.
struct poke_site { unsigned long addr; unsigned shift; };

static struct poke_site poke_site(unsigned long getter)
{
    unsigned long load = getter + PCMODE_LOAD_OFF;
    return (struct poke_site){ load & ~7UL, (unsigned)(load & 7) * 8 };
}

static uint32_t site_word(struct poke_site ps, unsigned long dw)
{
    return (uint32_t)(dw >> ps.shift);
}

static unsigned long site_with(struct poke_site ps, unsigned long dw, uint32_t w)
{
    return (dw & ~(0xffffffffUL << ps.shift)) | ((unsigned long)w << ps.shift);
}

/* Body of the getter, one aarch64 word per entry.  The two `bl` (mutex lock
 * and unlock) are compared on their opcode bits only. */
static const uint32_t PCMODE_SIG[] = {
    0xa9be7bfd, /* stp  x29, x30, [sp, #-0x20]! */
    0x910003fd, /* mov  x29, sp                 */
    0xa90153f3, /* stp  x19, x20, [sp, #0x10]   */
    0xaa0003f4, /* mov  x20, x0                 */
    0x9100c013, /* add  x19, x0, #0x30          */
    0xaa1303e0, /* mov  x0, x19                 */
    0x94000000, /* bl   <lock>                  */
    0xb9400a94, /* ldr  w20, [x20, #8]          */
    0xaa1303e0, /* mov  x0, x19                 */
    0x94000000, /* bl   <unlock>                */
    0x2a1403e0, /* mov  w0, w20                 */
    0xa94153f3, /* ldp  x19, x20, [sp, #0x10]   */
    0xa8c27bfd, /* ldp  x29, x30, [sp], #0x20   */
    0xd65f03c0, /* ret                          */
};
static const uint32_t PCMODE_MASK[] = {
    0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
    0xffffffff, 0xffffffff, 0xfc000000, 0xffffffff,
    0xffffffff, 0xfc000000, 0xffffffff, 0xffffffff,
    0xffffffff, 0xffffffff,
};
#define PCMODE_SIG_WORDS (sizeof PCMODE_SIG / sizeof PCMODE_SIG[0])

/* Scan the executable segment of EP122's binary for the getter.  Returns its
 * virtual address, or 0 on no match or more than one. */
static unsigned long find_pcmode_getter(pid_t pid)
{
    char path[64];
    snprintf(path, sizeof path, "/proc/%d/exe", (int)pid);
    FILE *f = fopen(path, "rb");
    if (!f) return 0;

    unsigned char ehdr[64];
    if (fread(ehdr, 1, sizeof ehdr, f) != sizeof ehdr) { fclose(f); return 0; }
    uint64_t phoff;   memcpy(&phoff,   ehdr + 0x20, sizeof phoff);
    uint16_t phentsz; memcpy(&phentsz, ehdr + 0x36, sizeof phentsz);
    uint16_t phnum;   memcpy(&phnum,   ehdr + 0x38, sizeof phnum);

    unsigned long found = 0;
    int matches = 0;
    for (uint16_t i = 0; i < phnum; i++) {
        unsigned char ph[56];
        if (fseek(f, (long)(phoff + (uint64_t)i * phentsz), SEEK_SET) != 0) break;
        if (fread(ph, 1, sizeof ph, f) != sizeof ph) break;
        uint32_t type, flags;
        uint64_t off, vaddr, fsz;
        memcpy(&type,  ph + 0x00, sizeof type);
        memcpy(&flags, ph + 0x04, sizeof flags);
        memcpy(&off,   ph + 0x08, sizeof off);
        memcpy(&vaddr, ph + 0x10, sizeof vaddr);
        memcpy(&fsz,   ph + 0x20, sizeof fsz);
        if (type != 1 /* PT_LOAD */ || !(flags & 1 /* PF_X */) || fsz < 4096) continue;

        uint32_t *text = malloc(fsz);
        if (!text) continue;
        if (fseek(f, (long)off, SEEK_SET) != 0 || fread(text, 1, fsz, f) != fsz) {
            free(text);
            continue;
        }
        size_t words = fsz / 4;
        for (size_t w = 0; w + PCMODE_SIG_WORDS <= words; w++) {
            size_t k = 0;
            while (k < PCMODE_SIG_WORDS &&
                   (text[w + k] & PCMODE_MASK[k]) == PCMODE_SIG[k]) k++;
            if (k == PCMODE_SIG_WORDS) {
                found = (unsigned long)(vaddr + (uint64_t)w * 4);
                if (++matches > 1) break;
            }
        }
        free(text);
        if (matches > 1) break;
    }
    fclose(f);

    if (matches != 1) {
        fprintf(stderr, "pc-link-bridge: PC-mode getter %s; not patching\n",
                matches ? "matched more than once" : "not found");
        return 0;
    }
    return found;
}

static pid_t find_pid(const char *comm)
{
    DIR *d = opendir("/proc");
    if (!d) return -1;
    struct dirent *e;
    pid_t found = -1;
    while ((e = readdir(d))) {
        if (e->d_name[0] < '0' || e->d_name[0] > '9') continue;
        char p[280]; snprintf(p, sizeof p, "/proc/%s/comm", e->d_name);
        FILE *f = fopen(p, "r");
        if (!f) continue;
        char nm[64] = "";
        if (fgets(nm, sizeof nm, f)) nm[strcspn(nm, "\n")] = 0;
        fclose(f);
        if (strcmp(nm, comm) == 0) { found = (pid_t)atoi(e->d_name); break; }
    }
    closedir(d);
    return found;
}

void force_pc_mode(void)
{
    pid_t pid = -1;
    for (int i = 0; i < 60 && pid < 0 && !g_stop; i++) {
        pid = find_pid("EP122");
        if (pid < 0) msleep(1000);
    }
    if (g_stop) return;
    if (pid < 0) {
        fprintf(stderr, "pc-link-bridge: EP122 not found; PC mode not forced\n");
        return;
    }
    unsigned long getter = find_pcmode_getter(pid);
    if (!getter) return;
    struct poke_site ps = poke_site(getter);
    if (ptrace(PTRACE_ATTACH, pid, 0, 0) != 0) {
        fprintf(stderr, "pc-link-bridge: ptrace attach %d failed: %s\n", pid, strerror(errno));
        return;
    }
    int st;
    waitpid(pid, &st, 0);
    errno = 0;
    long cur = ptrace(PTRACE_PEEKTEXT, pid, (void *)ps.addr, 0);
    if (cur == -1 && errno) {
        fprintf(stderr, "pc-link-bridge: PC-mode peek failed: %s\n", strerror(errno));
    } else if (site_word(ps, (unsigned long)cur) == EP122_PCMODE_PATCH) {
        fprintf(stderr, "pc-link-bridge: PC mode already forced\n");
    } else if (site_word(ps, (unsigned long)cur) != EP122_PCMODE_ORIG) {
        fprintf(stderr, "pc-link-bridge: PC-mode getter mismatch (0x%x); firmware differs, not patching\n",
                site_word(ps, (unsigned long)cur));
    } else if (ptrace(PTRACE_POKETEXT, pid, (void *)ps.addr,
                      (void *)site_with(ps, (unsigned long)cur, EP122_PCMODE_PATCH)) != 0) {
        fprintf(stderr, "pc-link-bridge: PC-mode poke failed: %s\n", strerror(errno));
    } else {
        fprintf(stderr, "pc-link-bridge: forced PC mode on EP122 pid %d (getter %#lx)\n",
                pid, getter);
    }
    ptrace(PTRACE_DETACH, pid, 0, 0);
}

/* Undo force_pc_mode() when the bridge stops (PC Link toggled off), so the deck
 * returns to standalone behaviour, the "unplugged cable" model.  Only reverts
 * bytes we ourselves patched. */
void unforce_pc_mode(void)
{
    pid_t pid = find_pid("EP122");
    if (pid < 0) return;
    unsigned long getter = find_pcmode_getter(pid);
    if (!getter) return;
    struct poke_site ps = poke_site(getter);
    if (ptrace(PTRACE_ATTACH, pid, 0, 0) != 0) return;
    int st;
    waitpid(pid, &st, 0);
    errno = 0;
    long cur = ptrace(PTRACE_PEEKTEXT, pid, (void *)ps.addr, 0);
    if (!(cur == -1 && errno) && site_word(ps, (unsigned long)cur) == EP122_PCMODE_PATCH) {
        if (ptrace(PTRACE_POKETEXT, pid, (void *)ps.addr,
                   (void *)site_with(ps, (unsigned long)cur, EP122_PCMODE_ORIG)) == 0)
            fprintf(stderr, "pc-link-bridge: restored PC-mode getter on EP122 pid %d\n", pid);
    }
    ptrace(PTRACE_DETACH, pid, 0, 0);
}
