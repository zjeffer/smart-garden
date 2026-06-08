use defmt::{error, info};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::gpio::Flex;
use onewire::{DeviceSearch, OneWire};
use core::sync::atomic::{AtomicI32, Ordering};

// Stores the last read temperature in centi-degrees (°C * 100).
// `i32::MIN` indicates no reading yet.
pub static LAST_TEMPERATURE_CENTI: AtomicI32 = AtomicI32::new(i32::MIN);

/// Read the Dallas DS18B20 temperature sensor (1-wire)
/// Code from https://github.com/kellerkindt/onewire/blob/master/examples/embassy_rp2040/src/main.rs
#[embassy_executor::task]
pub async fn read_temperature_sensor(mut wire: OneWire<Flex<'static>>) {
    'infinite: loop {
        // reset to test if wire is okay and if any sensor is connected
        if let Err(e) = wire.reset(&mut Delay) {
            error!("Failed to reset 1-wire bus: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
            continue 'infinite;
        }

        // search for devices on the bus
        let mut search = DeviceSearch::new();
        let Ok(device) = wire.search_next(&mut search, &mut Delay) else {
            error!("Temperature device search failed");
            continue 'infinite;
        };

        let Some(device) = device else {
            error!("No temperature device found");
            continue 'infinite;
        };

        let sensor = match onewire::ds18b20::DS18B20::new(device) {
            Ok(sensor) => sensor,
            Err(e) => {
                error!("Temperature device is not a DS18B20: {:?}", e);
                continue 'infinite;
            }
        };

        'measure: loop {
            let resolution = match sensor.measure_temperature(&mut wire, &mut Delay) {
                Ok(resolution) => resolution,
                Err(e) => {
                    error!("Failed to measure temperature: {:?}", e);
                    continue 'measure;
                }
            };

            // wait for measurement to complete
            Timer::after(Duration::from_millis(resolution.time_ms() as u64)).await;

            // get temp
            match sensor.read_temperature(&mut wire, &mut Delay) {
                Ok(temp) => {
                    let (integer, fraction) = onewire::ds18b20::split_temp(temp);
                    let temperature = (integer as f32) + (fraction as f32 / 10000.0);
                    // store centi-degrees to avoid floating point atomic issues
                    let centi = (temperature * 100.0) as i32;
                    LAST_TEMPERATURE_CENTI.store(centi, Ordering::Relaxed);
                    info!("Temperature: {} °C", temperature);
                }
                Err(e) => {
                    error!("Failed to read temperature: {:?}", e);
                    continue 'measure;
                }
            }
        }
    }
}
