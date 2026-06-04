use core::marker::Sized;
use core::{module_path, option_env};
use defmt::info;
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{Interface, WifiController};

/// This task handles connecting to Wi-Fi and reconnecting if the connection is lost. It will run indefinitely.
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    info!("Starting connection task...");

    loop {
        match controller.connect_async().await {
            Ok(info) => {
                let ssid =
                    core::str::from_utf8(info.ssid.as_str().as_bytes()).expect("<invalid ssid>");
                info!("Connected to Wi-Fi network! Info: {:?}", ssid);

                // wait for disconnect
                let info = controller.wait_for_disconnect_async().await;
                info!("Disconnected from Wi-Fi network! Info: {:?}", info);
                // next loop we'll try to reconnect again
            }
            Err(e) => {
                info!("Failed to connect to Wi-Fi network: {:#?}", e);
            }
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// This task runs the embassy-net network stack. It should be spawned once and will run indefinitely.
#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await;
}
