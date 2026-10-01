#!/bin/bash


cargo +esp build -r

espflash save-image \
       --chip esp32 \
       target/xtensa-esp32-none-elf/release/my-esp-project \
       firmware.bin