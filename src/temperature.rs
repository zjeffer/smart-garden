use defmt::{error, info};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::gpio::Flex;
use onewire::{DeviceSearch, OneWire};

/// Read the Dallas DS18B20 temperature sensor (1-wire)
/// Code from https://github.com/kellerkindt/onewire/blob/master/examples/embassy_rp2040/src/main.rs
#[embassy_executor::task]
pub async fn read_temperature_sensor(mut wire: OneWire<Flex<'static>>) {
	let mut delay = Delay;
    'infinite: loop {
        // reset to test if wire is okay and if any sensor is connected
        if let Err(e) = wire.reset(&mut delay) {
            error!("Failed to reset 1-wire bus: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
            continue 'infinite;
        }

        // search for devices on the bus
        let mut search = DeviceSearch::new();
        let Ok(device) = wire.search_next(&mut search, &mut delay) else {
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
