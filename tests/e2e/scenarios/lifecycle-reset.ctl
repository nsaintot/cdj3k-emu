# The reset before each scenario discards the eMMC writes of the one before,
# such as the marker that lifecycle-persist leaves.
#@ setting audio off
#@ setting network nat
#@ setting mods off
sh test ! -e /home/root/settings/e2e-marker
