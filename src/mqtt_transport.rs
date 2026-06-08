//! MQTT transport adapter (embassy-net TcpSocket -> embedded_io_async Read/Write)
//!
//! This small adapter provides a named transport type that can be passed to
//! `rust_mqtt::client::Client::connect`. `embassy-net`'s `TcpSocket` already
//! implements `embedded_io_async::Read`/`Write`, but this wrapper makes the
//! expected type explicit and can be extended later (TLS, buffering, etc.).

use embedded_io_async::{Read, Write, ErrorType};
use embassy_net::tcp::TcpSocket;

pub struct MqttTransport<'a> {
    pub socket: TcpSocket<'a>,
}

impl<'a> MqttTransport<'a> {
    pub fn new(socket: TcpSocket<'a>) -> Self {
        Self { socket }
    }
}

impl<'a> ErrorType for MqttTransport<'a> {
    type Error = embassy_net::tcp::Error;
}

impl<'a> Read for MqttTransport<'a> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.socket.read(buf).await
    }
}

impl<'a> Write for MqttTransport<'a> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.socket.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.socket.flush().await
    }
}
