# guest/modules/Makefile - build kernel modules inside Docker
#
# Builds the out-of-tree kernel modules for the Pioneer EP122 aarch64 guest:
#   subucom_virt.ko, virtio_snd.ko, udev_usb1.ko
#
# Usage:
#   make docker       - run docker/Dockerfile, outputs to build/docker-out/modules/

REPO_ROOT := $(shell git -C $(CURDIR) rev-parse --show-toplevel)

.PHONY: docker

docker:
	docker buildx build \
	  --platform linux/arm64 \
	  --target artifacts \
	  --output "type=local,dest=$(REPO_ROOT)/build/docker-out" \
	  -f "$(REPO_ROOT)/docker/Dockerfile" \
	  "$(REPO_ROOT)"
	@echo "Built: $(REPO_ROOT)/build/docker-out/modules/"
