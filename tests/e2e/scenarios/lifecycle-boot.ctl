# The deck boots to the SOURCE screen with no device, and its player app runs.
#@ setting audio off
#@ setting network nat
#@ setting mods off
sh systemctl is-active EP122.service
match main refs/{model}-{release}/source-none.png --region 0,0,0.72,0.25 --timeout 120000
wait lamp SOURCE on
