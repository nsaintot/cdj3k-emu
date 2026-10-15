# The guest mounts the USB image on attach and unmounts it on eject, and the
# SOURCE screen follows each change.
#@ media ../fixtures/e2e-usb.img
#@ setting audio off
#@ setting network nat
#@ setting mods off
wait lamp SLOT_2 136,240,255 120000
sh grep -q ' /media/usb/' /proc/mounts
match main refs/{model}-{release}/source-usb.png --region 0,0.13,0.72,0.25 --timeout 120000
media eject
sh for i in $(seq 50); do grep -q ' /media/usb/' /proc/mounts || exit 0; sleep 0.5; done; exit 1
match main refs/{model}-{release}/source-none.png --region 0,0.13,0.72,0.25 --timeout 120000
media attach {media}
wait lamp SLOT_2 136,240,255 120000
match main refs/{model}-{release}/source-usb.png --region 0,0.13,0.72,0.25 --timeout 120000
