#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 33: stop the deck flagging its own PC-link gadget as unsupported.
#
# 90-usb-caution.rules raises the "unsupported" caution (E-8307) for HID
# interfaces the deck does not expect. The PC-link gadget binds to dummy_hcd and
# enumerates on the deck's own USB bus, so the deck flags itself. Match the
# dummy_hcd controller in DEVPATH rather than a bus name: EP122 whitelists HID
# on 5-1 only, while EP145 whitelists none and uses 1-1 for its second port.
set -eu
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

RULES="$ROOTFS/etc/udev/rules.d/90-usb-caution.rules"
MARKER='ENV{INTERFACE}=="3/*", RUN+='
ADDED='ACTION=="add", DEVPATH=="*dummy_hcd*", ENV{INTERFACE}=="3/*", GOTO="usb_caution_end"'

if [ ! -f "$RULES" ]; then
    echo "  [33] 90-usb-caution.rules not present - skipping"
    exit 0
fi
if grep -qF 'dummy_hcd' "$RULES"; then
    echo "  [33] already whitelisted"
    exit 0
fi
if ! grep -qF "$MARKER" "$RULES"; then
    echo "  [33] WARNING: HID caution rule not found, rules left as shipped" >&2
    exit 0
fi

awk -v marker="$MARKER" -v added="$ADDED" '
    !done && index($0, marker) { print added; done = 1 }
    { print }
' "$RULES" > "$RULES.new" && mv "$RULES.new" "$RULES"

echo "  [33] dummy_hcd gadget HID whitelisted"
