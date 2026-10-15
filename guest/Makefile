# guest/Makefile builds the guest binaries (aarch64 Linux, run inside QEMU).
#
# Targets:
#   make tools        - builds the static tools with aarch64-linux-musl-gcc
#                       (Homebrew musl-cross); the shim needs make docker
#   make docker       - builds with docker/Dockerfile (arm64v8/alpine:3.19)
#   make abi-check    - checks out/deck_shim.so; under musl it inspects the
#                       Docker-built .so instead of relinking it
#
# The Dockerfile runs this Makefile inside the container, so the gcc
# invocations are defined only here. Override the compiler with
# `make CC=gcc <target>`; the Docker stages do this because they run natively
# on aarch64 and use the distribution's `gcc`.
#
# Layout (one directory per binary; sources live next to the binary they ship):
#   deck_shim/       The shim: time shift, jog, DRM and the syscall
#                    interposers. The cdj3k-mods ship as their own
#                    LD_PRELOADed object and are not built here.
#   subucom/         subucom_forwarder_aarch64 and subucom_live_aarch64.
#   cfgd/            cfgd_aarch64, the virtio-serial bridge for cdj3k.cfg.
#   pc_link_bridge/  pc_link_bridge_aarch64, the PC Link HID and MIDI bridge.
#   devd/            devd_aarch64, the Developer Mode daemon for cdj3k.dev.
#
# All outputs go to guest/out/.

MUSL_CC  := aarch64-linux-musl-gcc
HAS_MUSL := $(shell command -v $(MUSL_CC) 2>/dev/null)

ifdef HAS_MUSL
  CC := $(MUSL_CC)
else
  CC := aarch64-linux-gnu-gcc  # Fallback; use 'make docker' for a clean build.
endif

# `-mno-outline-atomics` is a GCC 10+ flag. Older GCCs (e.g. Ubuntu 18.04's
# 7.5) error out on it. Probe and add it only if the compiler accepts it.
NO_OUTLINE_ATOMICS := $(shell printf 'int main(void){return 0;}\n' \
    | $(CC) -x c -mno-outline-atomics -c -o /dev/null - 2>/dev/null \
    && echo -mno-outline-atomics)

OUT        := out
DECK_SHIM := $(OUT)/deck_shim.so

# The deck runs glibc 2.17 and carries exactly these libraries. abi-check
# asserts both against the linked shim.
GLIBC_MAX := 2.17
DECK_LIBS := libc.so.6 libm.so.6 libpthread.so.0 libdl.so.2 librt.so.1 \
             ld-linux-aarch64.so.1
READELF   ?= $(CC:gcc=readelf)

# -static-libgcc: link the libgcc helpers, including the outline-atomics
#   dispatchers on GCC 10+, into the .so, so it does not need libgcc_s on the
#   deck's glibc 2.17.
# -mno-outline-atomics: on GCC 10+, emit inline LL/SC atomics instead of
#   helper-function calls.
# -Wno-missing-field-initializers: the row and probe tables omit trailing
#   optional fields on purpose.
DECK_SHIM_CFLAGS := -O2 -Wall -Wextra -Wno-missing-field-initializers \
                     -Werror=implicit-function-declaration -fPIC -shared -static-libgcc \
                     $(NO_OUTLINE_ATOMICS) \
                     -finput-charset=utf-8 -fexec-charset=utf-8 \
                     -Ideck_shim/core

# The shim sources. The wildcards do not descend into subdirectories.
SHIM_CORE_SRCS := $(wildcard deck_shim/core/*.c) $(wildcard deck_shim/cdj3kx/*.c)
SHIM_CORE_OBJS := $(SHIM_CORE_SRCS:%.c=$(OUT)/%.o)
SHIM_CORE_HDRS := $(wildcard deck_shim/core/*.h)

# The tools link statically, so each binary is self-contained.
TOOL_CFLAGS := -O2 -Wall -Wextra -static

TOOL_TARGETS := $(OUT)/subucom_forwarder_aarch64 \
                $(OUT)/subucom_live_aarch64 \
                $(OUT)/cfgd_aarch64 \
                $(OUT)/pc_link_bridge_aarch64 \
                $(OUT)/devd_aarch64

CDJ_IP ?= 10.10.50.246
DEPLOY := /home/root/deck_shim.so

.PHONY: all shim tools clean deploy docker abi-check

all: $(OUT) $(DECK_SHIM) $(TOOL_TARGETS)

# These targets let the Dockerfile build only the shim or only the static
# tools without listing every output path.
shim:  $(OUT) $(DECK_SHIM)
tools: $(OUT) $(TOOL_TARGETS)

$(OUT):
	mkdir -p $(OUT)

# --- shim ---
# The shim is the only target that musl cannot build.
#
# A musl-linked .so needs libc.so, which the deck's glibc 2.17 does not have,
# so ld.so refuses the preload in every process that apl_start.sh starts,
# including /bin/sh, and the deck starts no app.
#
# The recipe refuses a musl CC because the default CC here is the musl cross
# compiler, which is the correct one for the static tools. `make docker`
# builds the shim in the ubuntu:18.04 stage against glibc 2.27.
$(DECK_SHIM): $(SHIM_CORE_OBJS)
	@case '$(CC)' in *musl*) \
	  echo "ERROR: $(CC) links the shim against musl (NEEDED: libc.so)."; \
	  echo "       The deck runs glibc 2.17, and its ld.so would refuse the shim"; \
	  echo "       and stop EP122 and /bin/sh. Build it with: make docker"; \
	  exit 1;; esac
	$(CC) $(DECK_SHIM_CFLAGS) -Wl,--no-undefined -o $@ $^ -lpthread
	@echo "Built: $@"
	@command -v file >/dev/null && file $@ || true

# Compile each shim source. No -fvisibility flag is set, so the hooks, such as
# open, read, clock_gettime and drmSetMaster, are exported.
$(OUT)/%.o: %.c $(SHIM_CORE_HDRS) | $(OUT)
	@mkdir -p $(dir $@)
	$(CC) $(DECK_SHIM_CFLAGS) -c -o $@ $<

# --- static tools ---
$(OUT)/subucom_forwarder_aarch64: subucom/forwarder.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $< -lpthread

$(OUT)/subucom_live_aarch64: subucom/live.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<

$(OUT)/cfgd_aarch64: cfgd/cfgd.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<

$(OUT)/devd_aarch64: devd/devd.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<


PC_LINK_SRCS := $(wildcard pc_link_bridge/*.c)
$(OUT)/pc_link_bridge_aarch64: $(PC_LINK_SRCS) pc_link_bridge/bridge.h | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $(PC_LINK_SRCS)

# --- gates ---
# The musl cross compiler cannot link the shim (see the guard above), so under
# musl the check inspects the .so that `make docker` left in out/.
# READELF is derived from CC, and the musl readelf reads a glibc .so.
ifeq ($(findstring musl,$(CC)),)
abi-check: $(DECK_SHIM)
else
abi-check: shim-artifact
.PHONY: shim-artifact
shim-artifact:
	@[ -f $(DECK_SHIM) ] || { \
	  echo "ERROR: no $(DECK_SHIM) to inspect. Build it first: make docker"; exit 1; }
	@echo "abi-check: inspecting the Docker-built $(DECK_SHIM)" \
	  "(`date -r $(DECK_SHIM) '+%Y-%m-%d %H:%M'`). Run 'make docker' to rebuild it."
endif
# The shim is LD_PRELOADed into every process apl_start.sh starts, all of them
# glibc 2.17. The check asserts the glibc version ceiling, the libc flavour,
# and that the deck carries every NEEDED library. If any of them fails, the
# deck starts no application.
abi-check:
	@fail=0; \
	echo "== ABI check: $(DECK_SHIM) against glibc $(GLIBC_MAX) =="; \
	vers=`$(READELF) --dyn-syms -W $(DECK_SHIM) \
	      | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV`; \
	echo "   glibc versions required: `echo $$vers | tr '\n' ' '`"; \
	for v in $$vers; do \
	  n=$${v#GLIBC_}; \
	  if [ "`printf '%s\n%s\n' "$$n" "$(GLIBC_MAX)" | sort -V | tail -1`" != "$(GLIBC_MAX)" ]; then \
	    echo "   FAIL: the shim needs $$v but the deck has glibc $(GLIBC_MAX); ld.so would refuse the preload."; fail=1; \
	    $(READELF) --dyn-syms -W $(DECK_SHIM) | grep "$$v" | awk '{print "         " $$8}' | sort -u; \
	  fi; \
	done; \
	needed=`$(READELF) -d $(DECK_SHIM) | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'`; \
	echo "   NEEDED: `echo $$needed | tr '\n' ' '`"; \
	case "$$needed" in *libc.so.6*) ;; \
	  *) echo "   FAIL: NEEDED has no libc.so.6, so this is probably a musl build; the deck uses glibc."; fail=1;; esac; \
	for l in $$needed; do \
	  case " $(DECK_LIBS) " in *" $$l "*) ;; \
	    *) echo "   FAIL: the deck does not carry the NEEDED library $$l."; fail=1;; esac; \
	done; \
	[ $$fail -eq 0 ] || { echo "abi-check: FAILED"; exit 1; }; \
	echo "   ok: the shim loads on glibc $(GLIBC_MAX)"

clean:
	rm -rf $(OUT)

deploy: $(DECK_SHIM)
	dd if=$(DECK_SHIM) | ssh root@$(CDJ_IP) "dd of=$(DEPLOY)"
	@echo "Deployed to $(CDJ_IP):$(DEPLOY)"

# Build with docker/Dockerfile (arm64v8/alpine:3.19, plus an ubuntu:18.04 stage
# for the shim). The build extracts the guest tools, the kernel and the modules
# to build/docker-out/, then copies the user-space outputs into guest/out/ for
# the packaging stagers.
docker:
	$(eval REPO_ROOT := $(shell git -C $(CURDIR) rev-parse --show-toplevel))
	$(eval DOCKER_OUT := $(REPO_ROOT)/build/docker-out)
	$(eval DOCKER_CACHE := $(REPO_ROOT)/build/docker-cache)
	$(eval DOCKER_BUILDER := $(shell "$(REPO_ROOT)/docker/ensure-buildx-builder.sh"))
	mkdir -p $(DOCKER_CACHE)
	docker buildx build \
	  --builder $(DOCKER_BUILDER) \
	  --platform linux/arm64 \
	  --provenance=false \
	  --target artifacts \
	  --cache-from type=local,src=$(DOCKER_CACHE) \
	  --cache-to type=local,dest=$(DOCKER_CACHE),mode=max \
	  --output "type=local,dest=$(DOCKER_OUT)" \
	  -f "$(REPO_ROOT)/docker/Dockerfile" \
	  "$(REPO_ROOT)"
	mkdir -p $(OUT)
	cp $(DOCKER_OUT)/deck_shim.so             $(OUT)/
	cp $(DOCKER_OUT)/subucom_forwarder_aarch64 $(OUT)/
	cp $(DOCKER_OUT)/subucom_live_aarch64      $(OUT)/
	cp $(DOCKER_OUT)/cfgd_aarch64              $(OUT)/
	cp $(DOCKER_OUT)/pc_link_bridge_aarch64    $(OUT)/
	cp $(DOCKER_OUT)/devd_aarch64              $(OUT)/
	@echo "Built via Docker: $(OUT)/"
