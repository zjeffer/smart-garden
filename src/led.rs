//! Onboard LED controller

use esp_hal::gpio::{DriveMode, Level, Output, OutputConfig};

pub async fn setup_led<'a>(gpio_pin: esp_hal::gpio::AnyPin<'a>) -> Output<'a> {
    let led_config = OutputConfig::default().with_drive_mode(DriveMode::PushPull);
    esp_hal::gpio::Output::new(gpio_pin, Level::Low, led_config)
}
