#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::info;
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::sta::StationConfig;
use panic_rtt_target as _;
extern crate alloc;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

use garden_esp::connection::{connection, net_task};
use garden_esp::led::setup_led;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// Put these in an .env file. See .env.example for an example.
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.3.0
    // generator parameters: --chip esp32c3 -o esp32c3-mini-1 -o unstable-hal -o alloc -o wifi -o embassy -o ble-trouble -o probe-rs -o defmt -o panic-rtt-target -o embedded-test -o vscode -o nightly-x86_64-unknown-linux-gnu

    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // The following pins are used to bootstrap the chip. They are available
    // for use, but check the datasheet of the module for more information on them.
    // - GPIO2
    // - GPIO8
    // - GPIO9
    // These GPIO pins are in use by some feature of the module and should not be used.
    let _ = peripherals.GPIO11;
    let _ = peripherals.GPIO12;
    let _ = peripherals.GPIO13;
    let _ = peripherals.GPIO14;
    let _ = peripherals.GPIO15;
    let _ = peripherals.GPIO16;
    let _ = peripherals.GPIO17;

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    let station_config = esp_radio::wifi::Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );

    info!("Initializing Wi-Fi controller...");
    let (mut wifi_controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("Failed to initialize Wi-Fi controller");
    info!("Wi-Fi controller initialized!");

    let wifi_interface = interfaces.station;
    let dhcp_config = embassy_net::Config::dhcpv4(Default::default());

    let rng = esp_hal::rng::Rng::new();
    let seed = (rng.random() as u64) << 32 | (rng.random() as u64);

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        dhcp_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    info!("Scanning for Wi-Fi networks...");
    let scan_config = ScanConfig::default().with_max(10); // max = max # of networks to return
    let scan_results = wifi_controller
        .scan_async(&scan_config)
        .await
        .expect("Failed to scan for Wi-Fi networks");
    if scan_results.is_empty() {
        info!("No Wi-Fi networks found");
    }
    for ap in scan_results {
        let ssid = core::str::from_utf8(ap.ssid.as_str().as_bytes()).expect("<invalid ssid>");
        info!("Found Wi-Fi network: AP info: {:?}", ssid);
    }

    spawner.spawn(connection(wifi_controller).expect("Failed to spawn connection task"));
    spawner.spawn(net_task(runner).expect("Failed to spawn net task"));

    stack.wait_config_up().await;

    // on-board led
    let mut led = setup_led(peripherals.GPIO8.into()).await;

    loop {
        info!("Blinking LED...");
        // blink esp32c3 onboard led every second
        led.set_high();
        Timer::after(embassy_time::Duration::from_millis(500)).await;
        led.set_low();
        Timer::after(embassy_time::Duration::from_millis(500)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples
}
