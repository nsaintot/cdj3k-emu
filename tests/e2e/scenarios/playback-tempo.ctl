# The tempo fader sets the pitch the play screen shows, at both ends and in
# the centre of the ±10 range.
#@ media ../fixtures/e2e-usb.img
#@ setting audio off
#@ setting network nat
#@ setting mods off
#@ needs hw-accel
#@ include lib/open-usb.inc
#@ include lib/load-a440.inc
tempo 1
match main refs/{model}-{release}/tempo-plus.png --region 0.69,0.71,0.82,0.8 --timeout 20000
tempo 0
match main refs/{model}-{release}/tempo-minus.png --region 0.69,0.71,0.82,0.8 --timeout 20000
tempo 0.5
match main refs/{model}-{release}/tempo-zero.png --region 0.69,0.71,0.82,0.8 --timeout 20000
