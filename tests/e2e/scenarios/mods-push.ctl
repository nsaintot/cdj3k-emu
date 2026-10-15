# A pushed mod's loader runs at boot and its variable reaches the player app.
# Disabled, it no longer runs.
#@ setting audio off
#@ setting network nat
#@ setting mods on
mods push ../fixtures/mods/e2e-hello
wait boot 600000
sh test -f /tmp/e2e-hello.ran
sh tr '\0' '\n' < /proc/$(pidof EP122)/environ | grep -qx E2E_HELLO=1
sh journalctl -b --no-pager -u cdj3k-mod-e2e-hello.service | grep -q 'e2e-hello: loader ran'
mods disable e2e-hello
restart
wait boot 600000
sh test ! -e /tmp/e2e-hello.ran
mods remove e2e-hello
