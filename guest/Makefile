# guest/Makefile - build all guest-side binaries (aarch64 Linux, runs inside QEMU)
#
# Two toolchain modes:
#   make              - expects aarch64-linux-musl-gcc (Homebrew musl-cross)
#   make docker       - builds via docker/Dockerfile (arm64v8/alpine:3.19)
#
# The Dockerfile drives this same Makefile inside the container — no
# duplicated gcc invocations on either side. Override the compiler with
# `make CC=gcc <target>` (Docker stages do this since they run natively
# on aarch64 and use the distro's plain `gcc`).
#
# Layout (one directory per binary; sources live next to the binary they ship):
#   ep122_shim/    → ep122_shim.so       (LD_PRELOAD shim, multi-TU shared object)
#   subucom/       → subucom_forwarder_aarch64, subucom_live_aarch64
#   cfgd/          → cfgd_aarch64        (cdj3k.cfg virtio-serial bridge)
#
# Outputs land under guest/out/ regardless of source directory.

MUSL_CC  := aarch64-linux-musl-gcc
HAS_MUSL := $(shell command -v $(MUSL_CC) 2>/dev/null)

ifdef HAS_MUSL
  CC := $(MUSL_CC)
else
  CC := aarch64-linux-gnu-gcc  # fallback - use 'make docker' for a clean build
endif

# `-mno-outline-atomics` is a GCC 10+ flag. Older GCCs (e.g. Ubuntu 18.04's
# 7.5) error out on it. Probe and add it only if the compiler accepts it.
NO_OUTLINE_ATOMICS := $(shell printf 'int main(void){return 0;}\n' \
    | $(CC) -x c -mno-outline-atomics -c -o /dev/null - 2>/dev/null \
    && echo -mno-outline-atomics)

# ep122_shim: position-independent shared library
# -static-libgcc: link the libgcc helpers (incl. outline-atomics dispatchers
#   on GCC 10+) into the .so so it doesn't need libgcc_s at runtime against
#   CDJ glibc 2.17.
# -mno-outline-atomics: belt-and-suspenders on GCC 10+ — emits inline LL/SC
#   atomics instead of helper-function calls (no helpers means nothing to
#   resolve even if libgcc were dynamic).
EP122_SHIM_CFLAGS := -O2 -Wall -fPIC -shared -static-libgcc \
                     $(NO_OUTLINE_ATOMICS) \
                     -finput-charset=utf-8 -fexec-charset=utf-8 -Iep122_shim

# Static tools: fully self-contained binaries
TOOL_CFLAGS := -O2 -Wall -Wextra -static

OUT  := out
EP122_SHIM := $(OUT)/ep122_shim.so

# ep122_shim translation units, listed by sub-directory.
EP122_SHIM_SRCS := ep122_shim/ep122_shim.c ep122_shim/fd.c ep122_shim/jog.c \
                   ep122_shim/jog_shm.c ep122_shim/jog_drm.c \
                   ep122_shim/syscalls.c ep122_shim/syscalls_io.c \
                   ep122_shim/clock.c ep122_shim/link.c
EP122_SHIM_OBJS := $(EP122_SHIM_SRCS:%.c=$(OUT)/%.o)
EP122_SHIM_HDRS := ep122_shim/ep122_shim.h

TOOL_TARGETS := $(OUT)/subucom_forwarder_aarch64 \
                $(OUT)/subucom_live_aarch64 \
                $(OUT)/cfgd_aarch64

CDJ_IP ?= 10.10.50.246
DEPLOY := /home/root/ep122_shim.so

.PHONY: all shim tools clean deploy docker

all: $(OUT) $(EP122_SHIM) $(TOOL_TARGETS)

# Convenience targets so the Dockerfile can request just the shim or just
# the static tools without listing every output path.
shim:  $(OUT) $(EP122_SHIM)
tools: $(OUT) $(TOOL_TARGETS)

$(OUT):
	mkdir -p $(OUT)

# --- shim ---
$(EP122_SHIM): $(EP122_SHIM_OBJS)
	$(CC) $(EP122_SHIM_CFLAGS) -o $@ $^
	@echo "Built: $@"
	@command -v file >/dev/null && file $@ || true

$(OUT)/%.o: %.c $(EP122_SHIM_HDRS) | $(OUT)
	@mkdir -p $(dir $@)
	$(CC) $(EP122_SHIM_CFLAGS) -c -o $@ $<

# --- static tools ---
$(OUT)/subucom_forwarder_aarch64: subucom/forwarder.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $< -lpthread

$(OUT)/subucom_live_aarch64: subucom/live.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<

$(OUT)/cfgd_aarch64: cfgd/cfgd.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<

clean:
	rm -rf $(OUT)

deploy: $(EP122_SHIM)
	dd if=$(EP122_SHIM) | ssh root@$(CDJ_IP) "dd of=$(DEPLOY)"
	@echo "Deployed to $(CDJ_IP):$(DEPLOY)"

# Build via docker/Dockerfile (arm64v8/alpine:3.19 + ubuntu:18.04 shim stage);
# extracts guest tools + kernel + modules to build/docker-out/, then copies
# the user-space outputs into guest/out/ for `bundle.sh` to pick up.
docker:
	$(eval REPO_ROOT := $(shell git -C $(CURDIR) rev-parse --show-toplevel))
	$(eval DOCKER_OUT := $(REPO_ROOT)/build/docker-out)
	docker buildx build \
	  --platform linux/arm64 \
	  --target artifacts \
	  --output "type=local,dest=$(DOCKER_OUT)" \
	  -f "$(REPO_ROOT)/docker/Dockerfile" \
	  "$(REPO_ROOT)"
	mkdir -p $(OUT)
	cp $(DOCKER_OUT)/ep122_shim.so             $(OUT)/
	cp $(DOCKER_OUT)/subucom_forwarder_aarch64 $(OUT)/
	cp $(DOCKER_OUT)/subucom_live_aarch64      $(OUT)/
	cp $(DOCKER_OUT)/cfgd_aarch64              $(OUT)/
	@echo "Built via Docker: $(OUT)/"
