#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::sta::StationConfig;
use garden_esp::temperature;
use panic_rtt_target as _;
extern crate alloc;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        STATIC_CELL.uninit().write($val)
    }};
}

use garden_esp::connection::{connection, net_task, scan_networks};
use garden_esp::led::setup_led;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// Put these in an .env file. See .env.example for an example.
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const TX_POWER: i8 = 50; // my esp32c3 doesn't work with the default (whatever that is), 50 works great.
const COUNTRY_INFO: &str = env!("COUNTRY_INFO");

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

    // Wi-Fi setup
    let station_config = esp_radio::wifi::Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );

    info!("Initializing Wi-Fi controller...");
    let country_code: [u8; 2] = COUNTRY_INFO.as_bytes()[..2]
        .try_into()
        .expect("Invalid country info, expected 2-character country code");
    let (mut wifi_controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default()
            .with_initial_config(station_config)
            .with_country_info(esp_radio::wifi::CountryInfo::from(country_code)),
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

    // required for one of my esp32's to be able to connect to Wi-Fi reliably
    wifi_controller
        .set_max_tx_power(TX_POWER)
        .expect("Failed to set max tx power");

    scan_networks(&mut wifi_controller).await;

    spawner.spawn(connection(wifi_controller).expect("Failed to spawn connection task"));
    spawner.spawn(net_task(runner).expect("Failed to spawn net task"));

    // will block until we have an IP address
    stack.wait_config_up().await;

    // print ip address
    let config = stack.config_v4();
    if let Some(config) = config {
        info!("Got IP address: {}", config.address);
    } else {
        warn!("No IPv4 address configured");
    }

    // spawn mqtt task (placeholder for now)
    spawner.spawn(garden_esp::mqtt::mqtt_task(stack).expect("Failed to spawn mqtt task"));

    // now that we have a connection, start reading the temperature sensor
    let mut data_pin = esp_hal::gpio::Flex::<'static>::new(peripherals.GPIO4);
    data_pin.apply_output_config(
        &esp_hal::gpio::OutputConfig::default()
            .with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain),
    );
    data_pin.set_output_enable(true);
    data_pin.set_input_enable(true);
    let wire = onewire::OneWire::new(data_pin, false);
    spawner.spawn(
        temperature::read_temperature_sensor(wire)
            .expect("Failed to spawn temperature sensor task"),
    );

    // on-board led
    let mut led = setup_led(peripherals.GPIO8.into()).await;

    loop {
        // blink esp32c3 onboard led
        led.set_high();
        Timer::after(embassy_time::Duration::from_millis(500)).await;
        led.set_low();
        Timer::after(embassy_time::Duration::from_millis(500)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples
}
