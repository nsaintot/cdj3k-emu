# SPDX-License-Identifier: MIT OR Apache-2.0
# CoreMIDI driver plugin. Needs no entitlement; installs per-user.
PLUGIN  := CDJ3KEmuMIDI.plugin
DEST    := $(HOME)/Library/Audio/MIDI Drivers
CFLAGS  ?= -O2 -Wall -Wextra
LDLIBS  := -framework CoreMIDI -framework CoreFoundation

SRCS    := plugin.c device.c link.c

# The runtime reads CDJ3KEmuMIDIVersion from the installed plist to decide
# whether that plugin still speaks its wire format.  It tracks IDENTITY_VERSION,
# the plugin's only external contract; a plist that disagrees with driver.h
# would make the check lie.
$(PLUGIN): $(SRCS) driver.h Info.plist
	@src=$$(sed -n 's/^#define IDENTITY_VERSION *//p' driver.h | tr -d ' '); \
	 plist=$$(sed -n 's/.*<key>CDJ3KEmuMIDIVersion<\/key><string>\(.*\)<\/string>.*/\1/p' Info.plist); \
	 if [ "$$src" != "$$plist" ]; then \
	     echo "IDENTITY_VERSION is $$src in driver.h but $$plist in Info.plist" >&2; exit 1; \
	 fi
	mkdir -p "$(PLUGIN)/Contents/MacOS"
	cp Info.plist "$(PLUGIN)/Contents/Info.plist"
	$(CC) $(CFLAGS) -bundle -o "$(PLUGIN)/Contents/MacOS/CDJ3KEmuMIDI" \
	    $(SRCS) $(LDLIBS)
	codesign --force --sign - "$(PLUGIN)"

install: $(PLUGIN)
	mkdir -p "$(DEST)"
	rm -rf "$(DEST)/$(PLUGIN)"
	cp -R "$(PLUGIN)" "$(DEST)/"
	killall MIDIServer 2>/dev/null || true
	@echo "installed; MIDIServer reloads on the next MIDI client"

uninstall:
	rm -rf "$(DEST)/$(PLUGIN)"
	killall MIDIServer 2>/dev/null || true

clean:
	rm -rf $(PLUGIN)

.PHONY: install uninstall clean
