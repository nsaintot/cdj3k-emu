# guest/Makefile - build all guest-side binaries (aarch64 Linux, runs inside QEMU)
#
# Two toolchain modes:
#   make              - expects aarch64-linux-musl-gcc (Homebrew musl-cross)
#   make docker       - builds via docker/Dockerfile (arm64v8/alpine:3.19)
#   make abi-check    - gates out/ep122_shim.so; under musl it inspects the
#                       Docker-built .so instead of relinking it
#
# The Dockerfile drives this same Makefile inside the container — no
# duplicated gcc invocations on either side. Override the compiler with
# `make CC=gcc <target>` (Docker stages do this since they run natively
# on aarch64 and use the distro's plain `gcc`).
#
# Layout (one directory per binary; sources live next to the binary they ship):
#   ep122_shim/    → the emulator half of the shim: time-shift, jog, DRM and
#                    the syscall interposers. Self-contained; the cdj3k-mods
#                    ship as their own package and are not built here.
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

# Version stamp, in the two forms the shim reports. Overridable from the
# command line: .dockerignore keeps .git out of the build context, so the `docker`
# target below passes the host's values in. No git checkout gives "unknown".
GIT_DESCRIBE := $(shell git -C $(CURDIR) describe --tags --always --dirty 2>/dev/null)
GIT_TAG      := $(shell git -C $(CURDIR) describe --tags --abbrev=0 2>/dev/null)
MOD_BUILD    ?= $(or $(GIT_DESCRIBE),unknown)
MOD_VERSION  ?= $(if $(GIT_TAG),$(GIT_TAG)$(if $(filter $(GIT_TAG),$(GIT_DESCRIBE)),,+),unknown)

OUT        := out
EP122_SHIM := $(OUT)/ep122_shim.so

# The deck runs glibc 2.17 and carries exactly these libraries. abi-check
# asserts both against the linked shim.
GLIBC_MAX := 2.17
DECK_LIBS := libc.so.6 libm.so.6 libpthread.so.0 libdl.so.2 librt.so.1 \
             ld-linux-aarch64.so.1
READELF   ?= $(CC:gcc=readelf)

# -static-libgcc: link the libgcc helpers (incl. outline-atomics dispatchers
#   on GCC 10+) into the .so so it doesn't need libgcc_s at runtime against
#   CDJ glibc 2.17.
# -mno-outline-atomics: belt-and-suspenders on GCC 10+ — emits inline LL/SC
#   atomics instead of helper-function calls.
# -Wno-missing-field-initializers: the row and probe tables omit trailing optional
#   fields on purpose.
EP122_SHIM_CFLAGS := -O2 -Wall -Wextra -Wno-missing-field-initializers \
                     -Werror=implicit-function-declaration -fPIC -shared -static-libgcc \
                     $(NO_OUTLINE_ATOMICS) \
                     -finput-charset=utf-8 -fexec-charset=utf-8 \
                     -DEP122_MOD_VERSION='"$(MOD_VERSION)"' \
                     -DEP122_MOD_BUILD='"$(MOD_BUILD)"' \
                     -Iep122_shim

# The emulator half: time-shift, jog, DRM and the syscall interposers. One
# level, and self-contained.
SHIM_CORE_SRCS := $(wildcard ep122_shim/*.c)
SHIM_CORE_OBJS := $(SHIM_CORE_SRCS:%.c=$(OUT)/%.o)
SHIM_CORE_HDRS := $(wildcard ep122_shim/*.h)

# Static tools: fully self-contained binaries
TOOL_CFLAGS := -O2 -Wall -Wextra -static

TOOL_TARGETS := $(OUT)/subucom_forwarder_aarch64 \
                $(OUT)/subucom_live_aarch64 \
                $(OUT)/cfgd_aarch64 \
                $(OUT)/pc_link_bridge_aarch64

# --- host tests ---
HOST_CC ?= cc
# The SDK headers carry nullability annotations and the sources call syscall(2),
# which Apple deprecated; neither says anything about the code under test.
HOST_QUIET := $(shell printf 'int main(void){return 0;}\n' \
    | $(HOST_CC) -x c -Wno-nullability-completeness -Wno-deprecated-declarations \
      -c -o /dev/null - 2>/dev/null \
    && echo -Wno-nullability-completeness -Wno-deprecated-declarations)
HOST_CFLAGS := -O1 -g -Wall -Wextra -Wno-missing-field-initializers $(HOST_QUIET) \
               -DEP122_MOD_VERSION='"$(MOD_VERSION)"' \
               -DEP122_MOD_BUILD='"$(MOD_BUILD)"'

CDJ_IP ?= 10.10.50.246
DEPLOY := /home/root/ep122_shim.so

.PHONY: all shim tools clean deploy docker abi-check

all: $(OUT) $(EP122_SHIM) $(TOOL_TARGETS)

# Convenience targets so the Dockerfile can request just the shim or just
# the static tools without listing every output path.
shim:  $(OUT) $(EP122_SHIM)
tools: $(OUT) $(TOOL_TARGETS)

$(OUT):
	mkdir -p $(OUT)

# --- shim ---
# The shim is the ONE target musl cannot build.
#
# It is LD_PRELOADed into EP122 and into every process apl_start.sh launches,
# all of them glibc 2.17. A musl-linked .so records `NEEDED: libc.so`, the deck
# has no musl at all, and glibc's ld.so then refuses to load it -- into /bin/sh
# as well, so apl_start.sh AND apl_end.sh die with status 127 and the service
# fails five times and gives up. The deck is left with no app and a log that
# says only "libc.so: cannot open shared object file".
#
# Hence the guard rather than a comment: the default CC here IS the musl cross
# compiler, because that is the right one for the static tools. `make docker`
# builds this in the ubuntu:18.04 stage against glibc 2.27.
$(EP122_SHIM): $(SHIM_CORE_OBJS)
	@case '$(CC)' in *musl*) \
	  echo "ERROR: $(CC) links the shim against musl (NEEDED: libc.so)."; \
	  echo "       The deck is glibc 2.17 and its ld.so will refuse it,"; \
	  echo "       taking EP122 and /bin/sh down with it. Use: make docker"; \
	  exit 1;; esac
	$(CC) $(EP122_SHIM_CFLAGS) -o $@ $^
	@echo "Built: $@"
	@command -v file >/dev/null && file $@ || true

# open, read, clock_gettime and drmSetMaster ARE the shim, so they link with
# default visibility and no prototype warning.
$(OUT)/%.o: %.c $(SHIM_CORE_HDRS) | $(OUT)
	@mkdir -p $(dir $@)
	$(CC) $(EP122_SHIM_CFLAGS) -c -o $@ $<

# --- static tools ---
$(OUT)/subucom_forwarder_aarch64: subucom/forwarder.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $< -lpthread

$(OUT)/subucom_live_aarch64: subucom/live.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<

$(OUT)/cfgd_aarch64: cfgd/cfgd.c | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $<


PC_LINK_SRCS := $(wildcard pc_link_bridge/*.c)
$(OUT)/pc_link_bridge_aarch64: $(PC_LINK_SRCS) pc_link_bridge/bridge.h | $(OUT)
	$(CC) $(TOOL_CFLAGS) -o $@ $(PC_LINK_SRCS)

# --- gates ---
# The musl cross compiler cannot link the shim (guard above), so under it the
# gate inspects the .so that `make docker` left in out/ rather than relinking.
# READELF derives from CC, and the musl readelf reads a glibc .so fine.
ifeq ($(findstring musl,$(CC)),)
abi-check: $(EP122_SHIM)
else
abi-check: shim-artifact
.PHONY: shim-artifact
shim-artifact:
	@[ -f $(EP122_SHIM) ] || { \
	  echo "ERROR: no $(EP122_SHIM) to inspect. Build it first: make docker"; exit 1; }
	@echo "abi-check: inspecting the Docker-built $(EP122_SHIM)" \
	  "(`date -r $(EP122_SHIM) '+%Y-%m-%d %H:%M'`); 'make docker' to rebuild."
endif
# The shim is LD_PRELOADed into every process apl_start.sh starts, all of them
# glibc 2.17: assert the version ceiling, the libc flavour, and that every
# NEEDED library is one the deck actually carries. Failing any of them leaves
# the deck with no application.
abi-check:
	@fail=0; \
	echo "== ABI check: $(EP122_SHIM) against glibc $(GLIBC_MAX) =="; \
	vers=`$(READELF) --dyn-syms -W $(EP122_SHIM) \
	      | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV`; \
	echo "   glibc versions required: `echo $$vers | tr '\n' ' '`"; \
	for v in $$vers; do \
	  n=$${v#GLIBC_}; \
	  if [ "`printf '%s\n%s\n' "$$n" "$(GLIBC_MAX)" | sort -V | tail -1`" != "$(GLIBC_MAX)" ]; then \
	    echo "   FAIL: needs $$v, deck has $(GLIBC_MAX) - ld.so would refuse the preload"; fail=1; \
	    $(READELF) --dyn-syms -W $(EP122_SHIM) | grep "$$v" | awk '{print "         " $$8}' | sort -u; \
	  fi; \
	done; \
	needed=`$(READELF) -d $(EP122_SHIM) | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'`; \
	echo "   NEEDED: `echo $$needed | tr '\n' ' '`"; \
	case "$$needed" in *libc.so.6*) ;; \
	  *) echo "   FAIL: no libc.so.6 in NEEDED (musl build? the deck is glibc)"; fail=1;; esac; \
	for l in $$needed; do \
	  case " $(DECK_LIBS) " in *" $$l "*) ;; \
	    *) echo "   FAIL: NEEDED $$l is not on the deck"; fail=1;; esac; \
	done; \
	[ $$fail -eq 0 ] || { echo "abi-check: FAILED"; exit 1; }; \
	echo "   ok: loadable on glibc $(GLIBC_MAX)"

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
	  --build-arg MOD_VERSION=$(MOD_VERSION) \
	  --build-arg MOD_BUILD=$(MOD_BUILD) \
	  --output "type=local,dest=$(DOCKER_OUT)" \
	  -f "$(REPO_ROOT)/docker/Dockerfile" \
	  "$(REPO_ROOT)"
	mkdir -p $(OUT)
	cp $(DOCKER_OUT)/ep122_shim.so             $(OUT)/
	cp $(DOCKER_OUT)/subucom_forwarder_aarch64 $(OUT)/
	cp $(DOCKER_OUT)/subucom_live_aarch64      $(OUT)/
	cp $(DOCKER_OUT)/cfgd_aarch64              $(OUT)/
	cp $(DOCKER_OUT)/pc_link_bridge_aarch64    $(OUT)/
	@echo "Built via Docker: $(OUT)/"
