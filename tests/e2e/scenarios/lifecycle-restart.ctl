# Three restarts in a row each bring the player app back, and systemd never
# restarts it.
#@ setting audio off
#@ setting network nat
#@ setting mods off
restart
wait boot 600000
sh systemctl is-active EP122.service
restart
wait boot 600000
sh systemctl is-active EP122.service
restart
wait boot 600000
sh systemctl is-active EP122.service
sh test "$(systemctl show -p NRestarts --value EP122.service)" = 0
match main refs/{model}-{release}/source-none.png --region 0,0,0.72,0.25 --timeout 120000
