# Two decks on one network: the second lists the first deck's USB drive over
# PRO DJ LINK, about 35 to 50 s after boot. The decks take their player
# numbers at boot, so the reference's mask leaves them out.
#@ slots 2
#@ media ../fixtures/e2e-usb.img
#@ setting audio off
#@ setting network host
#@ setting mods off
#@ needs hw-accel
slot 2
media eject
slot 1
wait lamp SLOT_2 136,240,255 120000
sh grep -q ' /media/usb/' /proc/mounts
slot 2
match main refs/{model}-{release}/link-usb.png --region 0,0.13,0.72,0.22 --timeout 240000 --settle 45000
