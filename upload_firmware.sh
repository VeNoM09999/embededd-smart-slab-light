#!/usr/bin/env bash

set -e

FIRMWARE="firmware.bin"
DEVICE="10.0.0.14"

if [[ ! -f "$FIRMWARE" ]]; then
    echo "Error: $FIRMWARE not found"
    exit 1
fi

SIZE=$(stat -c '%s' "$FIRMWARE")
CRC=$(python -c 'import zlib, sys; print(zlib.crc32(open(sys.argv[1], "rb").read()) & 0xffffffff)' "$FIRMWARE")

echo "Firmware: $FIRMWARE"
echo "Size:     $SIZE bytes"
echo "CRC32:    $CRC"
echo "Uploading to $DEVICE..."

curl --max-time 120 \
    -X POST \
    "http://${DEVICE}/update?size=${SIZE}&crc=${CRC}" \
    --data-binary "@${FIRMWARE}"

echo
echo "OTA upload complete."
