#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
// #![deny(clippy::large_stack_frames)]
#![allow(unused, dead_code, unused_variables)]
#![recursion_limit = "256"]

use alloc::sync::Arc;
use core::{
    net::Ipv4Addr,
    sync::atomic::{AtomicBool, AtomicU8},
    time,
};
use edge_nal::with_timeout;
use embassy_executor::Spawner;
use embassy_net::{IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer, WithTimeout};
use embedded_io_async::Read;
use esp_bootloader_esp_idf::partitions::RawPartitionType::App;
use esp_hal::system::Stack as esphal_Stack;
use esp_hal::{
    clock::CpuClock,
    gpio::{self, Output},
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, Channel, ChannelIFace},
        timer::{self, TimerIFace},
    },
    peripherals::{FLASH, GPIO25, GPIO26, GPIO27, Peripherals},
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::wifi::{ControllerConfig, Interface, WifiController, sta::StationConfig};
use esp_rtos::embassy::Executor;
use esp_storage::FlashStorage;
use heapless::String;
use my_esp_project::{
    LED_SIGNAL, OTA_CHANNEL, OTA_CHUNK_SIZE, OtaChunk, OtaCommand, RESTART_SIGNAL, SlabLightState,
    ota_task,
};
use picoserve::{
    extract::{FromRequest, Query},
    request::RequestBody,
    response::{
        Content, Json, WebSocketUpgrade,
        ws::{
            self, SpecifiedProtocol, UpgradedWebSocket, WebSocketCallback,
            WebSocketCallbackWithState,
        },
    },
    routing::post,
};
use picoserve::{
    extract::{FromRequestParts, State},
    response::{
        IntoResponse, IntoResponseWithState, Redirect, StatusCode, with_state::WithStateUpdate,
    },
    routing::get,
};
use serde::{Deserialize, Serialize};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

#[derive(Clone)]
struct AppState {
    device_name: &'static str,
    version: &'static str,
    led_on: Arc<AtomicBool>,
    white_bright: Arc<AtomicU8>,
    warm_bright: Arc<AtomicU8>,
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

const SSID: &str = "SSID";
const PASSWORD: &str = "PASSWORD";

static APP_CORE_STACK: static_cell::StaticCell<esphal_Stack<8192>> = static_cell::StaticCell::new();
static OTA_EXECUTOR: static_cell::StaticCell<Executor> = static_cell::StaticCell::new();
/// Main Entry Point
///
/// # Arguments
///
/// * `spawner` - Given by Embassy to spawn task on single core.
#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.3.0
    // generator parameters: --chip esp32s3 -o unstable-hal -o alloc -o wifi -o embassy -o vscode -o neovim -o esp

    // Initalizing all the peripherals, everything is accessible through this (gpio, etc)
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz);
    let peripherals = esp_hal::init(config);
    let flash = FlashStorage::new(peripherals.FLASH);

    let ledc = mk_static!(Ledc<'static>, Ledc::new(peripherals.LEDC));

    let gpio_tup = (peripherals.GPIO25, peripherals.GPIO26);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    // let access_point_config =
    //     esp_radio::wifi::Config::AccessPoint(AccessPointConfig::default().with_ssid("esp-radio"));

    let station_mode_config = esp_radio::wifi::Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );

    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // let (wifi_controller, interfaces) = esp_radio::wifi::new(
    //     peripherals.WIFI,
    //     ControllerConfig::default().with_initial_config(access_point_config),
    // )
    let wifi_per = peripherals.WIFI;

    let (wifi_controller, interfaces) = esp_radio::wifi::new(
        wifi_per,
        ControllerConfig::default().with_initial_config(station_mode_config),
    )
    .expect("Failed to initialize Wi-Fi controller");

    let device = interfaces.station;

    println!("Device MAC Addr: {:?}", device.mac_address());

    // let gw_ip_addr = Ipv4Addr::from_octets([192, 168, 2, 1]);

    let config = embassy_net::Config::dhcpv4(Default::default());

    let (stack, runner) = embassy_net::new(
        device,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        10_u64,
    );
    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(ensure_connected(wifi_controller, stack).unwrap());
    spawner.spawn(web_task(stack).unwrap());
    spawner.spawn(handle_led_signal(ledc, gpio_tup).unwrap());
    spawner.spawn(restart_task().unwrap());
    spawner.spawn(ota_task(flash).unwrap());

    loop {
        core::future::pending::<()>().await;
    }
}

struct OTAUpload;

#[derive(Deserialize)]
struct OTAParams {
    size: usize,
    crc: u32,
}
impl<'r, State> FromRequest<'r, State> for OTAUpload {
    type Rejection = (StatusCode, &'static str);

    async fn from_request<R: picoserve::io::Read>(
        mut state: &'r State,
        request_parts: picoserve::request::RequestParts<'r>,
        request_body: RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<OTAParams>::from_request_parts(state, &request_parts)
            .await
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OTA Params"))?;

        println!("OTA size={} crc={}", params.size, params.crc);

        let mut reader = request_body.reader();

        let mut buffer = [0u8; 8 * 1024];

        let sender = OTA_CHANNEL.sender();
        sender
            .send(OtaCommand::Start {
                size: params.size,
                crc: params.crc,
            })
            .await;

        println!("OTA Chunk Loop Started");
        let mut received = 0usize;
        let mut acc = [0u8; OTA_CHUNK_SIZE];
        let mut acc_len = 0usize;

        loop {
            let n = match reader.read(&mut buffer).await {
                Ok(n) => n,
                Err(e) => {
                    println!("OTA Read Failed");
                    sender.send(OtaCommand::Abort).await;
                    return Err((
                        picoserve::response::StatusCode::INTERNAL_SERVER_ERROR,
                        "OTA read failed",
                    ));
                }
            };

            if n == 0 {
                println!("HTTP BODY EOF, received={}", received);
                if acc_len > 0 {
                    let chunk = OtaChunk {
                        data: acc,
                        len: acc_len,
                    };
                    sender.send(OtaCommand::Chunk(chunk)).await;
                    acc_len = 0;
                }
                break;
            }
            received += n;
            let mut offset = 0usize;

            while offset < n {
                let space = OTA_CHUNK_SIZE - acc_len;
                let take = space.min(n - offset);

                acc[acc_len..acc_len + take].copy_from_slice(&buffer[offset..offset + take]);
                acc_len += take;
                offset += take;

                if acc_len == OTA_CHUNK_SIZE {
                    let chunk = OtaChunk {
                        data: acc,
                        len: acc_len,
                    };
                    sender.send(OtaCommand::Chunk(chunk)).await;
                    acc_len = 0;
                };
            }
        }
        println!("Total Received: {} bytes", received);
        sender.send(OtaCommand::Finish).await;

        Ok(OTAUpload)
    }
}

async fn ota(otaupload: OTAUpload) -> impl IntoResponse {
    "OTA uploaded"
}

static OTA_HTML: &str = include_str!("../../ota.html");
#[embassy_executor::task]
async fn restart_task() {
    RESTART_SIGNAL.wait().await;
    println!("Restarting...");

    // Give the TCP stack/picoserve time to finish sending the response.
    Timer::after_millis(100).await;

    esp_hal::system::software_reset();
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

const CMD_STATE: u8 = 0x01;
#[derive(Debug, Deserialize)]
enum WebSocketCommand {
    Set { warm: u8, white: u8 },
    Get,
    TurnOff,
}
struct WebSocketHandler {
    state: AppState,
}
impl WebSocketCallback for WebSocketHandler {
    async fn run<R: Read, W: embedded_io_async::Write<Error = R::Error>>(
        self,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        use picoserve::response::ws::Message;
        let state = self.state;
        let mut message_buffer = [0; 128];
        let close_reason: Option<(u16, &str)> = loop {
            let message = match rx
                .next_message(&mut message_buffer, core::future::pending::<()>())
                .await?
            {
                picoserve::futures::Either::First(Ok(result)) => result,
                picoserve::futures::Either::First(Err(_error)) => {
                    // log::warn!("WebSocket read error: {:?}", error);
                    // 1002 = Protocol Error
                    break Some((1002, "protocol error"));
                }
                picoserve::futures::Either::Second(_) => unreachable!(),
            };
            match message {
                Message::Close(reason) => {
                    // Echo back whatever the client asked to close with,
                    // or a normal-closure default if it didn't specify one.
                    break Some(reason.unwrap_or((1000, "closing")));
                }
                Message::Text(message) => {
                    let command: WebSocketCommand =
                        match serde_json_core::from_slice(message.as_bytes()) {
                            Ok((websockercommand, _size)) => websockercommand,
                            Err(_) => {
                                continue;
                            }
                        };
                    match command {
                        WebSocketCommand::Set { warm, white } => {
                            state
                                .white_bright
                                .store(white, core::sync::atomic::Ordering::Release);
                            state
                                .warm_bright
                                .store(warm, core::sync::atomic::Ordering::Release);

                            let led_on = white > 0 || warm > 0;

                            state
                                .led_on
                                .store(led_on, core::sync::atomic::Ordering::Release);
                            LED_SIGNAL.signal(SlabLightState {
                                white_brightness: white,
                                warm_brightness: warm,
                            });
                        }
                        WebSocketCommand::Get => {
                            let packet = [
                                CMD_STATE,
                                state.led_on.load(core::sync::atomic::Ordering::Relaxed) as u8,
                                state
                                    .white_bright
                                    .load(core::sync::atomic::Ordering::Relaxed),
                                state
                                    .warm_bright
                                    .load(core::sync::atomic::Ordering::Relaxed),
                            ];
                            tx.send_binary(&packet).await?;
                        }
                        WebSocketCommand::TurnOff => {
                            LED_SIGNAL.signal(SlabLightState {
                                white_brightness: 0,
                                warm_brightness: 0,
                            });
                        }
                    }
                }
                Message::Binary(_) => {}
                Message::Ping(ping) => {
                    tx.send_pong(ping).await?;
                }
                Message::Pong(_) => {}
            }
        };
        tx.close(close_reason).await
    }
}
async fn websocket(
    picoserve::extract::State(state): picoserve::extract::State<AppState>,
    upgrade: ws::WebSocketUpgrade,
) -> impl IntoResponseWithState<AppState> {
    upgrade.on_upgrade_using_state(WebSocketHandler {
        state: state.clone(),
    })
    // .with_protocol("messages")
}
#[embassy_executor::task]
async fn web_task(stack: Stack<'static>) {
    let mut rx_buffer = [0; 1536];
    let mut tx_buffer = [0; 1536];

    let app_state: &'static AppState = mk_static!(
        AppState,
        AppState {
            device_name: "Smart Slab Light",
            version: "0.0.9",
            led_on: Arc::new(AtomicBool::new(false)),
            warm_bright: Arc::new(AtomicU8::new(0)),
            white_bright: Arc::new(AtomicU8::new(0)),
        }
    );

    let app = picoserve::routing::Router::<_, AppState>::new()
        .route("/", get(|| async { "Hello World" }))
        .route("/status", get(status))
        .route("/set", get(set_brightness))
        .route("/update", post(ota))
        .route("/ota", get(ota_page))
        .route("/ws", get(websocket))
        .with_state(app_state);

    static CONFIG: picoserve::Config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Duration::from_secs(2),
        persistent_start_read_request: Duration::from_secs(2),
        read_request: Duration::from_secs(5),
        write: Duration::from_secs(5),
    })
    .keep_connection_alive();

    loop {
        let mut socket = embassy_net::tcp::TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        let _ = socket
            .accept(IpListenEndpoint {
                port: 80,
                ..Default::default()
            })
            .await;
        let remote_address = socket.remote_endpoint();
        match picoserve::Server::new(&app, &CONFIG, &mut [0; 2048])
            .serve(socket)
            .await
        {
            Ok(picoserve::DisconnectionInfo {
                handled_requests_count,
                ..
            }) => {
                println!("{handled_requests_count} requests handled from {remote_address:?}")
            }
            Err(err) => println!("{err:?}"),
        }
    }
}

#[embassy_executor::task]
async fn ensure_connected(mut wifi: WifiController<'static>, stack: Stack<'static>) {
    let mut to_show = false;
    loop {
        Timer::after(Duration::from_millis(500)).await;

        if !wifi.is_connected() {
            to_show = true;
            println!("Wifi Disconnected, Trying to reconnect...");
            let _ = wifi
                .connect_async()
                .with_timeout(Duration::from_secs(10))
                .await;
        } else if to_show {
            if stack.is_config_up() {
                to_show = false;
                stack
                    .config_v4()
                    .inspect(|c| println!("IPv4 config: {:?}", c));

                if let Some(config) = stack.config_v4() {
                    println!(
                        "Obtained IP Address: {:#?},{:#?}",
                        config.address, config.gateway
                    );
                    println!("Point your browser to http://{}", config.address.address());
                }
            } else if wifi.is_connected() && !stack.is_config_up() {
                println!("WIFI Connected; Obtaining IP Address...");
                stack
                    .wait_config_up()
                    .with_timeout(Duration::from_secs(10))
                    .await;
            }
        }
    }
}

#[embassy_executor::task]
async fn handle_led_signal(
    ledc: &'static mut Ledc<'static>,
    gpio: (GPIO25<'static>, GPIO26<'static>),
) {
    println!("Starting handle_led_signal task");
    SlabLightState::listen_for_events(ledc, gpio).await;
}

#[derive(Serialize)]
struct ApiResponse {
    message: &'static str,
    white_brightness: u8,
    warm_brightness: u8,
}

#[derive(Serialize, Deserialize)]
struct BrightnessParams {
    white: u8,
    warm: u8,
}
// Handlers
async fn set_brightness(
    State(state): State<AppState>,
    Query(params): Query<BrightnessParams>,
) -> impl IntoResponse {
    state
        .white_bright
        .store(params.white, core::sync::atomic::Ordering::Release);
    state
        .warm_bright
        .store(params.warm, core::sync::atomic::Ordering::Release);

    let led_on = params.white > 0 || params.warm > 0;

    state
        .led_on
        .store(led_on, core::sync::atomic::Ordering::Release);

    LED_SIGNAL.signal(SlabLightState {
        white_brightness: params.white,
        warm_brightness: params.warm,
    });
    Json(ApiResponse {
        message: "Success",
        warm_brightness: params.warm,
        white_brightness: params.white,
    })
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    use core::fmt::Write;

    let mut response: String<128> = String::new();
    let _ = write!(
        response,
        "Device Name:{}\r\nVersion: {}\r\nLed: {}\r\nWhite Brightness: {}\r\nWarm Brightness: {}",
        state.device_name,
        state.version,
        if state.led_on.load(core::sync::atomic::Ordering::Acquire) {
            "ON"
        } else {
            "OFF"
        },
        state
            .white_bright
            .load(core::sync::atomic::Ordering::Acquire),
        state
            .warm_bright
            .load(core::sync::atomic::Ordering::Acquire)
    );

    response
}

struct Html(&'static str);

impl Content for Html {
    fn content_type(&self) -> &'static str {
        "text/html; charset=utf-8"
    }
    fn content_length(&self) -> usize {
        self.0.len()
    }
    async fn write_content<W: embedded_io_async::Write>(
        self,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.write_all(self.0.as_bytes()).await
    }
}

async fn ota_page() -> impl IntoResponse {
    Html(OTA_HTML)
}
