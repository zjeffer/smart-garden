use core::marker::Sized;
use core::{module_path, option_env};
use defmt::{error, info, warn};
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::WifiError;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::{Interface, WifiController};

/// This task handles connecting to Wi-Fi and reconnecting if the connection is lost. It will run indefinitely.
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    info!(
        "Starting connection task, will try to connect to '{:?}'",
        env!("SSID")
    );

    loop {
        match controller.connect_async().await {
            Ok(info) => {
                let ssid =
                    core::str::from_utf8(info.ssid.as_str().as_bytes()).expect("<invalid ssid>");
                info!("Connected to Wi-Fi network! Info: {:?}", ssid);

                // wait for disconnect
                let info = controller.wait_for_disconnect_async().await;
                warn!("Disconnected from Wi-Fi network! Info: {:?}", info);
                // next loop we'll try to reconnect again
            }
            Err(e) => match e {
                WifiError::Disconnected(info) => {
                    let ssid = core::str::from_utf8(info.ssid.as_str().as_bytes())
                        .expect("<invalid ssid>");
                    let reason = info.reason;
                    error!(
                        "Disconnected from Wi-Fi network with SSID '{:?}', Reason: {:?}",
                        ssid, reason
                    );
                }
                WifiError::Unsupported
                | WifiError::InvalidArguments
                | WifiError::Failed
                | WifiError::OutOfMemory
                | WifiError::InvalidSsid
                | WifiError::InvalidPassword
                | WifiError::NotConnected
                | _ => {
                    error!("Failed to connect to Wi-Fi network: {:?}", e);
                }
            },
        }
        Timer::after(Duration::from_secs(2)).await;
    }
}

/// This task runs the embassy-net network stack. It should be spawned once and will run indefinitely.
#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await;
}

/// Scans for Wi-Fi networks and logs the results. Useful for debugging.
pub async fn scan_networks(wifi_controller: &mut WifiController<'static>) {
    info!("Scanning for Wi-Fi networks...");
    let scan_config = ScanConfig::default().with_max(10); // max = max # of networks to return
    let scan_results = wifi_controller
        .scan_async(&scan_config)
        .await
        .expect("Failed to scan for Wi-Fi networks");
    if scan_results.is_empty() {
        info!("No Wi-Fi networks found");
    } else {
        for ap in scan_results {
            let ssid = core::str::from_utf8(ap.ssid.as_str().as_bytes()).expect("<invalid ssid>");
            info!("Found Wi-Fi network: AP info: {:?}", ssid);
        }
    }
}
