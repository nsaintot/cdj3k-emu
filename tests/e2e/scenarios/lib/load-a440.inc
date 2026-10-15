# From the FOLDER view at the USB root, load Tracks/A440 120bpm.wav.
rotary 1
match main refs/{model}-{release}/folder-root-tracks.png --region 0.07,0.2,0.57,0.41 --timeout 30000
press ROTARY_PRESS
match main refs/{model}-{release}/tracks.png --region 0.15,0,0.55,0.12 --timeout 30000
rotary 1
match main refs/{model}-{release}/tracks-a440.png --region 0.07,0.2,0.57,0.41 --timeout 30000
press ROTARY_PRESS
match main refs/{model}-{release}/play-a440.png --region 0.14,0,0.42,0.07 --timeout 60000
