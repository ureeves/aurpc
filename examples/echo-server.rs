use aurpc::RpcSocket;

#[async_std::main]
async fn main() -> std::io::Result<()> {
    let socket = RpcSocket::bind("0.0.0.0:0").await?;

    let addr = socket.local_addr();
    println!("listening on: {:?}", addr);

    let mut buf = [0u8; 256];

    loop {
        let (bytes_read, responder) = socket.recv_from(&mut buf[..]).await?;
        responder.respond(&buf[..bytes_read]).await?;
    }
}
