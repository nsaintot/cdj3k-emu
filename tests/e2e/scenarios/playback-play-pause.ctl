# A loaded track shows its tempo and starts playing. PLAY pauses it and
# plays it again, and its remaining time runs only while it plays.
#@ media ../fixtures/e2e-usb.img
#@ setting audio off
#@ setting network nat
#@ setting mods off
#@ needs hw-accel
#@ include lib/open-usb.inc
#@ include lib/load-a440.inc
match main refs/{model}-{release}/bpm-a440.png --region 0.84,0.67,0.98,0.8 --timeout 60000
changes main 0.34,0.69,0.57,0.8 --timeout 20000
press PLAY
sleep 500
still main 0.34,0.69,0.57,0.8 --for 1500
press PLAY
changes main 0.34,0.69,0.57,0.8 --timeout 20000
