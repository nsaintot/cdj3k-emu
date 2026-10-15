# What the guest writes to the eMMC lasts across a restart.
#@ setting audio off
#@ setting network nat
#@ setting mods off
sh echo e2e > /home/root/settings/e2e-marker && sync
restart
wait boot 600000
sh test "$(cat /home/root/settings/e2e-marker)" = e2e
