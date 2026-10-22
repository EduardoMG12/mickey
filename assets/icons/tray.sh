#!/bin/sh
# Makes the tray pictures Mickey embeds (PRD 0009): the line-art rat at
# 32x32, as raw RGBA bytes (row by row, 4 bytes a pixel), so Mickey needs
# no image crate. The strokes are thickened first, or they fade at this
# size. Run from the repository root after changing the icon.
set -e
for colour in white black; do
	magick assets/icons/option-01.png -channel A -morphology Dilate Disk:9 +channel \
		-fill "$colour" -colorize 100 -resize 30x30 -background none -gravity center \
		-extent 32x32 -depth 8 "RGBA:crates/mickey/assets/tray-$colour.rgba"
done
