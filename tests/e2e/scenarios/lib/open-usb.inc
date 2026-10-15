# From the SOURCE screen, open the USB drive's FOLDER view at its root.
# The deck lights its USB slot, then fills the info panel once it has read
# the drive.
wait lamp SLOT_2 136,240,255 120000
match main refs/{model}-{release}/source-usb.png --region 0,0.13,1,0.5 --timeout 120000
press ROTARY_PRESS
# The list shows once the "rekordbox Database not found!" notice has gone.
match main refs/{model}-{release}/browse-root.png --region 0.07,0.2,0.57,0.41 --timeout 60000 --settle 8000
# The first press moves the focus into the list; the rotary works after it.
press ROTARY_PRESS
match main refs/{model}-{release}/folder-root.png --region 0.15,0,0.55,0.12 --timeout 30000
