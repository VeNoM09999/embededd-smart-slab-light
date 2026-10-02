# Smart SLab Light ESP32 Firmware

A lightweight, embedded Rust firmware for the **ESP32** microprocessor that provides smart LED light control with WiFi connectivity, web interface, and over-the-air (OTA) updates.

## Overview

This project implements a smart lighting system based from the following key features:

- **Smart LED Light Control**: Dual-channel PWM control for warm and white brightness adjustments using ESP32's hardware LED controllers
- **WiFi Connectivity**: Automatic reconnection with DHCP support
- **Web Interface**: HTTP REST API and WebSocket interface for real-state control
- **Over-The-Air (OTA) Updates**: Full firmware update capability with CRC verification
- **Resource-Constrained Design**: Optim for embedded devices with minimal resource requirements

## Hardware Requirements

- **ESP32** microprocessor
- Dual-channel PWM support via LED controllers (`ledc` peripheral)
- GPIO channels: `GPIO25` (white brightness), `GPIO26` (warm brightness)
- WiFi controller for network connectivity
- Flash storage for OTA partition management

## Features

### 1. Smart LED Light Control

- Dual-channel PWM control using ESP32's low-speed LED controllers
- Separate controls for warm and white light channels
- Real-state updates via WebSocket
- HTTP REST API for brightness configuration
- Signal-based state synchronization with async task handling

### 2. WiFi & Networking

- **Station mode** with automatic reconnection
- **DHCPv4** support via embassy-net
- TCP socket server using smoltcp stack
- Web interface accessible at `http://{device-ip}`
- WebSocket connection for real-state updates

### 3. Web Interface

REST API endpoints:
```
GET /status    - Device status (name, version, LED state)
GET /set?white={value}&warm={value}   - Set brightness levels
POST /update   - OTA firmware update endpoint
GET /ota       - HTML interface for OTA updates
```

WebSocket endpoint:
```
WS /ws        - Real-state WebSocket for live monitoring and control
```

### 4. Over-The-Air (OTA) Updates

- Partition-based flashing using esp-bootloader-esp-idf
- CRC32 verification for data integrity
- Chunked transfer with 4KB buffer size
- Automatic partition activation after successful update
- System restart on completion

## Get Started

### Prerequisites

- Rust toolchain (`rustup`)
- Target: `xtensa-esp32-none-elf` (ESP32 embedded target)
- cargo-embedded-flash for flashing
- Required dependencies in `.cargo/config.toml`

### Build Instructions

```bash
# Set the target:
cargo build --target xtensa-esp32-none-elf --release

# Flash to ESP32:
cargo flash --target xtensa-esp32-none-elf --release

# Monitor during boot:
cargo run --target xtensa-esp32-none-elf --release -- monitor
```

### Configuration

#### .cargo/config.toml

```toml
[xtenna-esp32-none-elf]
runner = "espflash flash --monitor --chip esp32 --partition-table ./partitions.csv --erase-parts otadata"

[target.xtensa-esp32-none-elf]
rustflags = [ "-C", "link-arg=-noststartfiles" ]
target = "xtenna-esp32-none-elf"
build-std = ["alloc", "core"]
```

#### Cargo.toml Configuration

The project uses:
- **ESP32** platform with WiFi support (`esp-radio` crate)
- embassy-net for networking stack
- smoltcp for TCP implementation
- picoserve for HTTP/WS server
- esp-bootloader-esp-idf for OTA updates
- embedded-hal and heapless for resource-constrained design

### Setting up the Project

1. **Configure WiFi SSID and Password** in `src/bin/main.rs`:
```rust
const SSID: &str = "your_ssid";
const PASSWORD: &str = "your_password";
```

2. **Partition Table**: Ensure you have `partitions.csv` for OTA partition management.

3. **Dependencies**: Verify all required crates in `Cargo.toml`.

## Architecture Overview

### Main Tasks (`src/bin/main.rs`)

1. `net_task`: Network stack runner using embassy-net
2. `ensure_connected`: WiFi reconnection logic with IP address reporting
3. `web_task`: HTTP and WebSocket server using picoserve
4. `handle_led_signal`: LED control task via LED controllers
5. `ota_task`: OTA update manager for firmware updates

### Key Components (`src/lib.rs`)

1. **SlabLightState**: Dual brightness state (warm + white)
2. **LED_SIGNAL**: Async signal-based state synchronization
3. **OtaChunk / OtaCommand**: Chunked data handling for updates
4. **CRC32 Update**: Data integrity verification during OTA

## API Reference

### HTTP REST API

```json
GET /status
// Response: Device name, version, LED state, brightness values

GET /set?white=100&warm=50
// Set both channels to specified brightness levels (0-8)

POST /update --with file body
// OTA firmware update via POST endpoint

GET /ota
// HTML interface for manual OTA updates
```

### WebSocket Commands

```json
{ "command": "Set", "warm": 50, "white": 100 } // Set brightness
{ "command": "Get" }                                - Read current state
{ "command": "TurnOff" } // LED off signal
```

## Build Profiles

### Dev Profile (Opt for size):
```toml
[profile.dev]
opt-level = "s"  // Size optimizations
```

### Release Profile:
```toml
[profile.release]
codegen-units = 1   // Single thread for better optimizations
debug = 2           // Better debugging experience
lto = 'fat'        // Link-time optimization
opt-level = 's'    // Size-based optimization
```

## Key Dependencies (Cargo.toml)

- `esp-hal`: Hardware abstraction layer
- `esp-rtos`: Real operating system for ESP32
- `esp-radio`: WiFi controller
- `embassy-net`: Network stack with DHCPv4 support
- `smoltcp`: TCP/IP implementation
- `picoserve`: HTTP/WS server
- `esp-bootloader-esp-idf`: OTA partition management
- `heapless`: Resource-constrained heap

## File Structure

```
wifi-pwm-ota/
    .cargo/  config.toml         - Target and build configuration
    Cargo.toml   - Project manifest
    README.md   - Documentation
    src/
        bin/main.rs  - Main entry point (tasks, server, web interface)
        lib.rs      - Library code (LED control, OTA handling)
        ota.html    - HTML interface for OTA updates
```

## Quick Usage Examples

### Set Brightness via HTTP:
```bash
curl "http://{ip}/set?white=100&warm=50"
```

### Get Status via HTTP:
```bash
curl "http://{ip}/status"
```

### OTA Update via Web Interface:
1. Access `GET /ota` from the device's web interface
2. Upload firmware file to `/update` endpoint
3. Device will verify, flash, and restart on success

## Configuration Examples

### Station Mode (with SSID/password)
```rust
station_mode_config = esp_radio::wifi::Config::Station(
StationConfig::default().with_ssid("SSID").with_password("PASSWORD"),
)
```

### Access Point Mode
```rust
// Optional: Set up as access point for clients to connect
esp_radio::wifi::Config::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio"))
```

## System Resources

- **Heap Size**: 73.74KB (`esp_alloc::heap_allocator!`)
- **Stack Size**: ~8KB (`esphal_Stack<8192>`)
- **CPU Clock**: 80MHz (config default)
- **Memory Layout**: Reclaimed heap region for flexibility

## Running and Monitoring

```bash
# Build:
cargo build --target xtensa-esp32-none-elf --release

# Flash and run with monitor:
cargo flash --target xtensa-esp32-none-elf --release

# Run with serial output via espflash:
cargo run --target xtenna-esp32-none-elf --release
```

## Example Usage Flow

1. Device boots, initializes peripherals (LED controllers, WiFi)
2. Connects to WiFi network (reconnection logic ensures stability)
3. DHCP assigns IP address
4. Web server starts on port 80
5. LED control task waits for signals from WebSocket/HTTP
7. Optional: OTA update via `/update` endpoint

## Future Enhancements

- [ ] Add access point mode for standalone operation
- [ ] Add more network protocols (multicast, DNS)
- [ ] Support for multiple brightness profiles
- [ ] Add logging configuration options
- [ ] Web interface improvements for OTA management

## Installation

```bash
# 1. Clone the repository:
git clone https://github.com/{username}/wifi-pwm-ota.git

# 2. Set up target:
rustup target add xtensa-esp32-none-elf

# 3. Build and flash:
cargo build --target xtensa-esp32-none-elf --release
cargo flash --target xtenna-esp32-none-elf --release
```

## Example Configuration

See `Cargo.toml` for all dependencies and profile settings. The project uses a custom toolchain (`rust-toolchain.toml`) with nightly compiler optimizations.

## Credits

Based from the following technologies:
- **ESP32** (Emperor of Silicon)
- **embassy-net**: Network stack
- **smoltcp**: TCP/IP implementation
- **picoserve**: HTTP/WS server
- **esp-bootloader-esp-idf**: OTA partition management

## License

**Non-commercial**: Free for anyone to use and customize. Attribution required when you use it (credit us in your project).

**Commercial**: If you want to use this firmware for commercial purposes, you must either:
1. Pay a license fee - Contact me via Discord DM for pricing
2. Or credit me with attribution (username: _v3n0m)

Contact my username **_v3n0m** on Discord for the best option that works for you!
```

---

**Note**: This firmware is designed for resource-constrained embedded devices with minimal overhead while providing powerful features like smart LED control, network connectivity, and over-the-air updates.
