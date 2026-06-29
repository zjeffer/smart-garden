//! Minimal MQTT integration using `rust_mqtt`.
//!
//! This module contains a small `mqtt_task` stub used while iterating on
//! adding a real MQTT publisher. The full client requires a transport and
//! buffer provider; those will be added in follow-ups.

extern crate alloc;

use defmt::{error, info};
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::TcpSocket;

use rust_mqtt::types::MqttBinary;
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

const TOPIC_GROUND_TEMPERATURE: &str = env!("TOPIC_GROUND_TEMPERATURE");
const BROKER_HOST: &str = env!("MQTT_BROKER_HOST");
const BROKER_PORT: &str = env!("MQTT_BROKER_PORT");
const BROKER_USERNAME: &str = env!("MQTT_BROKER_USERNAME");
const BROKER_PASSWORD: &str = env!("MQTT_BROKER_PASSWORD");

// Use `option_env!` at call sites so these values are optional at compile time.

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

    let port = BROKER_PORT.parse::<u16>().unwrap_or(1883);

    // Backoff / timing constants
    const RECONNECT_BASE_SECS: u64 = 5;
    const RECONNECT_MAX_SECS: u64 = 60;
    const TELEMETRY_INTERVAL_SECS: u64 = 30;

    let mut backoff = RECONNECT_BASE_SECS;

    // Outer loop: manage connection lifecycle. On any fatal error, wait and retry.
    loop {
        // Clear buffers before creating a new socket.
        let rx_buf = MQTT_TCP_RX_CELL.uninit().write([0u8; 1536]);
        let tx_buf = MQTT_TCP_TX_CELL.uninit().write([0u8; 1536]);

        info!("Resolving MQTT broker host: {}", BROKER_HOST);

        // Resolve broker host via DNS (A records). Embassy's DNS client is used
        // here because `TcpSocket::connect` expects an `IpEndpoint`.
        let dns = DnsSocket::new(stack);
        let query_res = dns.query(BROKER_HOST, DnsQueryType::A).await;

        let mut connected_ok = false;

        if let Ok(addrs) = query_res {
            if let Some(addr) = addrs.first() {
                let endpoint = IpEndpoint::new(*addr, port);
                info!("Connecting to MQTT broker: {:?}", endpoint);

                // Create a fresh socket for this attempt. The socket/transport
                // will be consumed by the client on connect, so recreate each try.
                let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);

                match socket.connect(endpoint).await {
                    Ok(()) => {
                        info!("TCP connected to broker");

                        let transport = MqttTransport::new(socket);

                        // Create a fresh client + buffer for this connection attempt.
                        let mut buffer = AllocBuffer;
                        let mut client: Client<
                            '_,
                            MqttTransport<'static>,
                            AllocBuffer,
                            1,
                            16,
                            16,
                            4,
                        > = Client::new(&mut buffer);

                        // Build connect options, applying username/password only when present.
                        let mut connect_opts =
                            rust_mqtt::client::options::ConnectOptions::new().clean_start();
                        if !BROKER_USERNAME.is_empty() {
                            connect_opts = connect_opts
                                .user_name(MqttString::from_str_unchecked(BROKER_USERNAME));
                        }
                        if !BROKER_PASSWORD.is_empty() {
                            connect_opts = connect_opts.password(MqttBinary::from_bytes_unchecked(
                                BROKER_PASSWORD.as_bytes().into(),
                            ));
                        }

                        match client.connect(transport, &connect_opts, None).await {
                            Ok(_) => {
                                info!("MQTT CONNECT succeeded");

                                // publish discovery as retained message. If this fails
                                // treat the whole attempt as failed and reconnect.
                                let disc_topic =
                                    "homeassistant/sensor/esp32c3_ground_temp_001/config";
                                let disc_topic_name = TopicName::new_unchecked(
                                    MqttString::from_str_unchecked(disc_topic),
                                );
                                let disc_opts =
                                    PublicationOptions::new(TopicReference::Name(disc_topic_name))
                                        .retain();

                                let boxed = discovery.clone().into_bytes().into_boxed_slice();
                                let msg = Bytes::from(boxed);
                                match client.publish(&disc_opts, msg).await {
                                    Ok(_) => {
                                        info!("Published discovery message");
                                        connected_ok = true;
                                        backoff = RECONNECT_BASE_SECS; // reset backoff
                                    }
                                    Err(e) => {
                                        error!("Discovery publish failed: {:?}", e);
                                        // let client/transport drop and fallthrough to reconnect
                                    }
                                }

                                // Enter publish loop while connected_ok is true.
                                while connected_ok {
                                    let last = LAST_TEMPERATURE_CENTI.load(Ordering::Relaxed);
                                    if last != i32::MIN {
                                        let temp = (last as f32) / 100.0; // convert back to degrees
                                        let payload =
                                            alloc::format!(r#"{{"temperature":{:.2}}}"#, temp);

                                        // prepare publication options for the telemetry topic
                                        let topic_name = TopicName::new_unchecked(
                                            MqttString::from_str_unchecked(
                                                TOPIC_GROUND_TEMPERATURE,
                                            ),
                                        );
                                        let opts = PublicationOptions::new(TopicReference::Name(
                                            topic_name,
                                        ));

                                        let boxed = payload.into_bytes().into_boxed_slice();
                                        let msg = Bytes::from(boxed);

                                        match client.publish(&opts, msg).await {
                                            Ok(_) => info!("Published temperature: {} °C", temp),
                                            Err(e) => {
                                                error!("Publish failed: {:?}", e);
                                                // Break publish loop and trigger reconnect
                                                connected_ok = false;
                                                break;
                                            }
                                        }
                                    } else {
                                        info!("No temperature reading yet; skipping publish");
                                    }

                                    // Wait before publishing again
                                    embassy_time::Timer::after(embassy_time::Duration::from_secs(
                                        TELEMETRY_INTERVAL_SECS,
                                    ))
                                    .await;
                                }
                            }
                            Err(e) => error!("MQTT CONNECT failed: {:?}", e),
                        }
                    }
                    Err(e) => error!("TCP connect failed: {:?}", e),
                }
            } else {
                error!("DNS returned no addresses for broker host: {}", BROKER_HOST);
            }
        } else {
            error!("DNS query failed for {}", BROKER_HOST);
        }

        if !connected_ok {
            info!("MQTT connect attempt failed; backing off {}s", backoff);
            embassy_time::Timer::after(embassy_time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(RECONNECT_MAX_SECS);
        } else {
            // If we exited the publish loop due to a publish error, reconnect immediately
            info!("MQTT disconnected; reconnecting in {}s", backoff);
            embassy_time::Timer::after(embassy_time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(RECONNECT_MAX_SECS);
        }
    }
}
