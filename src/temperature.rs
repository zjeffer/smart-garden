use core::sync::atomic::{AtomicI32, Ordering};
use defmt::{error, info, warn};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::gpio::Flex;
use onewire::{DeviceSearch, OneWire};

// Stores the moving average of the last up to 10 successful temperature
// readings in centi-degrees (°C * 100). `i32::MIN` indicates no reading yet.
pub static LAST_TEMPERATURE_CENTI: AtomicI32 = AtomicI32::new(i32::MIN);

/// Read the Dallas DS18B20 temperature sensor (1-wire)
/// Code from https://github.com/kellerkindt/onewire/blob/master/examples/embassy_rp2040/src/main.rs
#[embassy_executor::task]
pub async fn read_temperature_sensor(mut wire: OneWire<Flex<'static>>) {
    // Circular buffer (fixed-size) to keep the last up to 10 successful
    // measurements in centi-degrees. We maintain `buf_len` to know how
    // many valid samples are present (0..=10) and `buf_pos` for the next
    // insertion index.
    let mut buf: [i32; 10] = [0; 10];
    let mut buf_len: usize = 0;
    let mut buf_pos: usize = 0;

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

                    // Spike detection: if we have a previous valid sample,
                    // reject readings that differ by more than 5°C.
                    if buf_len > 0 {
                        let last_idx = if buf_pos == 0 {
                            buf.len() - 1
                        } else {
                            buf_pos - 1
                        };
                        let prev = buf[last_idx];
                        let diff = centi - prev;
                        let diff_abs = if diff < 0 { -diff } else { diff };
                        if diff_abs > 5 * 100 {
                            warn!(
                                "Spike detected: measured {} °C ({} centi), skipping",
                                temperature, centi
                            );
                            continue 'measure;
                        }
                    }

                    // push into circular buffer
                    buf[buf_pos] = centi;
                    buf_pos = (buf_pos + 1) % buf.len();
                    if buf_len < buf.len() {
                        buf_len += 1;
                    }

                    // compute average over the valid samples and update the
                    // atomic so other tasks (e.g. MQTT) can read it.
                    let sum: i64 = buf[..buf_len].iter().map(|&v| v as i64).sum();
                    let avg = (sum / (buf_len as i64)) as i32;
                    LAST_TEMPERATURE_CENTI.store(avg, Ordering::Relaxed);

                    info!(
                        "Temperature: {} °C (measured), moving avg: {} °C",
                        temperature,
                        (avg as f32) / 100.0
                    );
                }
                Err(e) => {
                    error!("Failed to read temperature: {:?}", e);
                    continue 'measure;
                }
            }
        }
    }
}
