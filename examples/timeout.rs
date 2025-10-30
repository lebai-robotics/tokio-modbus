use std::{net::SocketAddr, pin::Pin, time::Duration};

use futures::future;
use tokio::net::TcpListener;

use tokio_modbus::{
    prelude::*,
    server::tcp::{accept_tcp_connection, Server},
};

struct ExampleService {}

impl tokio_modbus::server::Service for ExampleService {
    type Request = Request;
    type Response = Response;
    type Error = std::io::Error;
    type Future =
        Pin<Box<dyn future::Future<Output = Result<Self::Response, Self::Error>> + Send + Sync>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        match req {
            Request::ReadHoldingRegisters(_addr, cnt) => {
                let data = vec![0; cnt as usize];
                Box::pin(future::ready(Ok(Response::ReadHoldingRegisters(data))))
            }
            Request::WriteMultipleRegisters(addr, values) => Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(Response::WriteMultipleRegisters(addr, values.len() as u16))
            }),
            Request::WriteSingleRegister(addr, value) => Box::pin(future::ready(Ok(
                Response::WriteSingleRegister(addr, value),
            ))),
            _ => {
                println!("SERVER: Exception::IllegalFunction - Unimplemented function code in request: {req:?}");
                // TODO: We want to return a Modbus Exception response `IllegalFunction`. https://github.com/slowtec/tokio-modbus/issues/165
                Box::pin(future::ready(Err(std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    "Unimplemented function code in request".to_string(),
                ))))
            }
        }
    }
}

impl ExampleService {
    fn new() -> Self {
        Self {}
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let socket_addr = "127.0.0.1:5502".parse().unwrap();

    tokio::select! {
        _ = server_context(socket_addr) => unreachable!(),
        _ = client_context(socket_addr) => println!("Exiting"),
    }

    Ok(())
}

async fn server_context(socket_addr: SocketAddr) -> anyhow::Result<()> {
    println!("Starting up server on {socket_addr}");
    let listener = TcpListener::bind(socket_addr).await?;
    let server = Server::new(listener);
    let new_service = |_socket_addr| Ok(Some(ExampleService::new()));
    let on_connected = |stream, socket_addr| async move {
        accept_tcp_connection(stream, socket_addr, new_service)
    };
    let on_process_error = |err| {
        eprintln!("{err}");
    };
    server.serve(&on_connected, on_process_error).await?;
    Ok(())
}

async fn client_context(socket_addr: SocketAddr) {
    let task = async {
        // Give the server some time for starting up
        tokio::time::sleep(Duration::from_secs(1)).await;

        println!("CLIENT: Connecting client...");
        let mut ctx = tcp::connect(socket_addr).await.unwrap();

        let read_reg = tokio::time::timeout(
            Duration::from_millis(200),
            ctx.read_holding_registers(0x01, 2),
        )
        .await;
        println!("{:?}", read_reg); // Should success
        assert!(read_reg.unwrap().is_ok());

        let write_mul = tokio::time::timeout(
            Duration::from_millis(200),
            ctx.write_multiple_registers(0x01, &[0]),
        )
        .await;
        println!("{:?}", write_mul); // Should timeout
        assert!(write_mul.is_err());
        tokio::time::sleep(Duration::from_millis(500)).await; // wait finish

        let write_sig = tokio::time::timeout(
            Duration::from_millis(200),
            ctx.write_single_register(0x01, 0),
        )
        .await;
        println!("{:?}", write_sig); // Should success
        assert!(write_sig.unwrap().is_ok());

        println!("CLIENT: Done.")
    };
    tokio::join!(task, tokio::time::sleep(Duration::from_secs(5)));
}
