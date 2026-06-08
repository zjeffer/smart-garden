//! Minimal MQTT integration using `rust_mqtt`.
//!
//! This module contains a small `mqtt_task` stub used while iterating on
//! adding a real MQTT publisher. The full client requires a transport and
//! buffer provider; those will be added in follow-ups.

extern crate alloc;

use defmt::info;
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::TcpSocket;

use smoltcp::wire::DnsQueryType;

use crate::mqtt_transport::MqttTransport;
use crate::temperature::LAST_TEMPERATURE_CENTI;
use core::sync::atomic::Ordering;
use rust_mqtt::Bytes;
use rust_mqtt::buffer::AllocBuffer;
use rust_mqtt::client::Client;
use rust_mqtt::client::options::{PublicationOptions, TopicReference};
use rust_mqtt::types::{MqttString, TopicName};

// Use `static_cell` to safely provide `'static` buffers for the TCP socket.
static MQTT_TCP_RX_CELL: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
static MQTT_TCP_TX_CELL: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();

const TOPIC_GROUND_TEMPERATURE: &str = "garden/ground_temperature";

/// Try to connect to the broker and publish a retained discovery message,
/// then return. This is `async` so the caller can spawn it as a task.
#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>) {
    info!("MQTT task: preparing discovery payload");

    let discovery = alloc::format!(
        r#"{{"name":"ESP32C3 Ground Temperature","state_topic":"{}","unit_of_measurement":"°C","value_template":"{{{{ value_json.temperature }}}}","unique_id":"esp32c3_ground_temp_001","device":{{"identifiers":["esp32c3_001"],"name":"ESP32C3","manufacturer":"You","model":"esp32c3-mini"}}}}"#,
        TOPIC_GROUND_TEMPERATURE
    );

    info!("MQTT discovery: {}", discovery.as_str());

    // instantiate the client buffer and client (transport type explicit so generics are known)
    let mut buffer = AllocBuffer;
    let mut client: Client<'_, MqttTransport<'static>, AllocBuffer, 1, 16, 16, 4> =
        Client::new(&mut buffer);
    let mut connected = false;

    // Broker resolution/connect sequence is optional at compile time. Set
    // `MQTT_BROKER_HOST` via `--env MQTT_BROKER_HOST=...` or in your build
    // environment to enable connecting. `MQTT_BROKER_PORT` may override port.
    if let Some(broker_host) = option_env!("MQTT_BROKER_HOST") {
        let port: u16 = option_env!("MQTT_BROKER_PORT")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(1883);

        // Initialize static cell backed buffers. `uninit().write(...)` returns
        // a `&'static mut [u8; 1536]` which coerces to `&'static mut [u8]`.
        let rx_buf: &'static mut [u8] = MQTT_TCP_RX_CELL.uninit().write([0u8; 1536]);
        let tx_buf: &'static mut [u8] = MQTT_TCP_TX_CELL.uninit().write([0u8; 1536]);

        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);

        // Resolve broker host via DNS (A records). Embassy's DNS client is used
        // here because `TcpSocket::connect` expects an `IpEndpoint`.
        let dns = DnsSocket::new(stack);
        match dns.query(broker_host, DnsQueryType::A).await {
            Ok(addrs) => {
                if let Some(addr) = addrs.get(0) {
                    let endpoint = IpEndpoint::new(*addr, port);
                    info!("Connecting to MQTT broker: {:?}", endpoint);

                    match socket.connect(endpoint).await {
                        Ok(()) => {
                            info!("TCP connected to broker");

                            let transport = MqttTransport::new(socket);

                            let connect_opts =
                                rust_mqtt::client::options::ConnectOptions::new().clean_start();

                            match client.connect(transport, &connect_opts, None).await {
                                Ok(_) => {
                                    info!("MQTT CONNECT succeeded");
                                    connected = true;

                                    // publish discovery as retained message
                                    let disc_topic =
                                        "homeassistant/sensor/esp32c3_ground_temp_001/config";
                                    let disc_topic_name = TopicName::new_unchecked(
                                        MqttString::from_str_unchecked(disc_topic),
                                    );
                                    let disc_opts = PublicationOptions::new(TopicReference::Name(
                                        disc_topic_name,
                                    ))
                                    .retain();

                                    let boxed = discovery.into_bytes().into_boxed_slice();
                                    let msg = Bytes::from(boxed);
                                    match client.publish(&disc_opts, msg).await {
                                        Ok(_) => info!("Published discovery message"),
                                        Err(e) => info!("Discovery publish failed: {:?}", e),
                                    }
                                }
                                Err(e) => info!("MQTT CONNECT failed: {:?}", e),
                            }
                        }
                        Err(e) => info!("TCP connect failed: {:?}", e),
                    }
                } else {
                    info!("DNS returned no addresses for broker host: {}", broker_host);
                }
            }
            Err(e) => info!("DNS query failed for {}: {:?}", broker_host, e),
        }
    } else {
        info!("MQTT_BROKER_HOST not set; skipping connect");
    }
    loop {
        if connected {
            let last = LAST_TEMPERATURE_CENTI.load(Ordering::Relaxed);
            if last != i32::MIN {
                let temp = (last as f32) / 100.0;
                let payload = alloc::format!(r#"{{"temperature":{:.2}}}"#, temp);

                // prepare publication options for the telemetry topic
                let topic_name = TopicName::new_unchecked(MqttString::from_str_unchecked(
                    TOPIC_GROUND_TEMPERATURE,
                ));
                let opts = PublicationOptions::new(TopicReference::Name(topic_name));

                let boxed = payload.into_bytes().into_boxed_slice();
                let msg = Bytes::from(boxed);

                match client.publish(&opts, msg).await {
                    Ok(_) => info!("Published temperature: {} °C", temp),
                    Err(e) => info!("Publish failed: {:?}", e),
                }
            } else {
                info!("No temperature reading yet; skipping publish");
            }
        } else {
            info!("Not connected to MQTT broker; skipping publish");
        }

        // Wait before publishing again
        embassy_time::Timer::after(embassy_time::Duration::from_secs(30)).await;
    }
}
