# SPDX-License-Identifier: MIT OR Apache-2.0
# Builds the CoreMIDI driver plugin. The plugin needs no entitlement and
# installs per user.
PLUGIN  := CDJ3KEmuMIDI.plugin
DEST    := $(HOME)/Library/Audio/MIDI Drivers
CFLAGS  ?= -O2 -Wall -Wextra
# packaging/macos/stage.sh passes one -arch flag per slice.
ARCHFLAGS ?=
LDLIBS  := -framework CoreMIDI -framework CoreFoundation

SRCS    := plugin.c device.c link.c

# The runtime reads CDJ3KEmuMIDIVersion from the installed plist to decide
# whether that plugin still speaks its wire format.  It tracks IDENTITY_VERSION,
# the plugin's only external contract, so the build fails when the plist and
# driver.h disagree.
$(PLUGIN): $(SRCS) driver.h Info.plist
	@src=$$(sed -n 's/^#define IDENTITY_VERSION *//p' driver.h | tr -d ' '); \
	 plist=$$(sed -n 's/.*<key>CDJ3KEmuMIDIVersion<\/key><string>\(.*\)<\/string>.*/\1/p' Info.plist); \
	 if [ "$$src" != "$$plist" ]; then \
	     echo "IDENTITY_VERSION is $$src in driver.h but $$plist in Info.plist" >&2; exit 1; \
	 fi
	mkdir -p "$(PLUGIN)/Contents/MacOS"
	cp Info.plist "$(PLUGIN)/Contents/Info.plist"
	$(CC) $(CFLAGS) $(ARCHFLAGS) -bundle -o "$(PLUGIN)/Contents/MacOS/CDJ3KEmuMIDI" \
	    $(SRCS) $(LDLIBS)
	codesign --force --sign - "$(PLUGIN)"

install: $(PLUGIN)
	mkdir -p "$(DEST)"
	rm -rf "$(DEST)/$(PLUGIN)"
	cp -R "$(PLUGIN)" "$(DEST)/"
	killall MIDIServer 2>/dev/null || true
	@echo "Installed. MIDIServer loads the new plugin when the next MIDI client starts."

uninstall:
	rm -rf "$(DEST)/$(PLUGIN)"
	killall MIDIServer 2>/dev/null || true

clean:
	rm -rf $(PLUGIN)

.PHONY: install uninstall clean
