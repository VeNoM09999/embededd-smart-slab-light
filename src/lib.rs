#![no_std]
#![allow(unused, dead_code, unused_variables)]

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::publisher;
use embassy_sync::signal::Signal;
use embedded_hal::pwm::SetDutyCycle;
use embedded_storage::Storage;
use esp_bootloader_esp_idf::ota::Ota;
use esp_hal::gpio;
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::TimerIFace;
use esp_hal::ledc::{HighSpeed, LSGlobalClkSource, Ledc, LowSpeed, timer};
use esp_hal::peripherals::{BT, GPIO25, GPIO26};
use esp_hal::time::{Instant, Rate};
use esp_println::println;
use esp_storage::FlashStorage;

#[derive(Debug)]
pub struct SlabLightState {
    pub white_brightness: u8,
    pub warm_brightness: u8,
}

pub static LED_SIGNAL: Signal<CriticalSectionRawMutex, SlabLightState> = Signal::new();

impl SlabLightState {
    pub async fn listen_for_events(
        ledc: &'static mut Ledc<'static>,
        gpio: (GPIO25<'static>, GPIO26<'static>),
    ) {
        ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

        let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
        let mut timer1 = ledc.timer::<LowSpeed>(timer::Number::Timer1);

        timer0
            .configure(timer::config::Config {
                duty: timer::config::Duty::Duty8Bit,
                clock_source: timer::LSClockSource::APBClk,
                frequency: Rate::from_khz(4),
            })
            .unwrap();

        timer1.configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: esp_hal::time::Rate::from_khz(4),
        });

        let mut white = ledc.channel::<LowSpeed>(channel::Number::Channel1, gpio.0);
        white.configure(channel::config::Config {
            timer: &timer1,
            duty_pct: 0,
            drive_mode: gpio::DriveMode::PushPull,
        });

        let mut warm = ledc.channel::<LowSpeed>(channel::Number::Channel0, gpio.1);
        warm.configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: gpio::DriveMode::PushPull,
        });
        loop {
            let channel = LED_SIGNAL.wait().await;
            println!("Got Signal: {:?}", channel);

            white.set_duty_cycle(channel.white_brightness.into());
            warm.set_duty_cycle(channel.warm_brightness.into());
        }
    }
}

use embassy_sync::channel::Channel;

pub const OTA_CHUNK_SIZE: usize = 4 * 1024;

pub struct OtaChunk {
    pub len: usize,
    pub data: [u8; OTA_CHUNK_SIZE],
}

pub enum OtaCommand {
    Start { size: usize, crc: u32 },
    Chunk(OtaChunk),
    Finish,
    Abort,
}

pub static OTA_CHANNEL: Channel<CriticalSectionRawMutex, OtaCommand, 2> = Channel::new();
pub static RESTART_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
#[embassy_executor::task]
pub async fn ota_task(mut flash: FlashStorage<'static>) {
    let receiver = OTA_CHANNEL.receiver();
    println!("OTA Task Started...");
    use esp_bootloader_esp_idf::{
        ota::OtaImageState,
        ota_updater::OtaUpdater,
        partitions::{self, PARTITION_TABLE_MAX_LEN},
    };

    loop {
        match receiver.receive().await {
            OtaCommand::Start {
                size,
                crc: expected_crc,
            } => {
                let mut pt_buffer = [0u8; PARTITION_TABLE_MAX_LEN];
                let pt = match partitions::read_partition_table(&mut flash, &mut pt_buffer) {
                    Ok(pt) => pt,
                    Err(_) => {
                        println!("Failed to read partitions table ");
                        continue;
                    }
                };

                println!("Currently booted: {:?}", pt.booted_partition());

                let mut ota_buffer = [0u8; PARTITION_TABLE_MAX_LEN];
                let mut ota = match OtaUpdater::new(&mut flash, &mut ota_buffer) {
                    Ok(ota) => ota,
                    Err(_) => {
                        println!("Failed to init OTAUpdater");
                        continue;
                    }
                };
                let (mut writer, part_type) = match ota.next_partition() {
                    Ok(w) => w,
                    Err(_) => {
                        println!("Failed to get next partitions");
                        continue;
                    }
                };
                println!("Flashing to {:?}, expecting {} bytes", part_type, size);

                let mut offset: u32 = 0;
                let mut running_crc: u32 = 0;
                let mut had_error = false;

                'inner: loop {
                    match receiver.receive().await {
                        OtaCommand::Chunk(chunk) => {
                            let data = &chunk.data[..chunk.len];
                            let time = Instant::now();
                            if writer.write(offset, data).is_err() {
                                println!("OTA Write failed at offset {offset} ");
                                had_error = true;
                                break 'inner;
                            }
                            println!("Time for write to flash: {}", time.elapsed());

                            running_crc = crc32_update(running_crc, data);
                            offset += data.len() as u32;
                        }
                        OtaCommand::Finish => break 'inner,
                        OtaCommand::Abort => {
                            had_error = true;
                            break 'inner;
                        }
                        OtaCommand::Start { .. } => {
                            println!("Unexpected Start mid-transfer, aborting");
                            had_error = true;
                            break 'inner;
                        }
                    }
                }

                if had_error || offset != size.try_into().unwrap() || running_crc != expected_crc {
                    println!(
                        "OTA verify failed: written={offset} expected_size={size} crc={running_crc:#x} expected_crc={expected_crc:#x}"
                    );
                    continue;
                }
                if let Err(_) = ota.activate_next_partition() {
                    println!("Failed to activate new partition");
                    continue;
                }
                let _ = ota.set_current_ota_state(OtaImageState::New);
                println!("OTA finalized — restarting");
                RESTART_SIGNAL.signal(());
            }
            _ => {}
        }
    }
}
