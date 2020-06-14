use async_std::io;
use std::env;
use udp_rpc::RpcSocket;

#[async_std::main]
async fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "needs a socket address as input",
        ));
    }

    let buf = b"hello, world";
    let mut rsp_buf = *buf;
    let socket = RpcSocket::bind("0.0.0.0:0").await?;

    let (_, read) = socket
        .send(&buf[..], &mut rsp_buf[..], args[1].clone())
        .await?;

    println!("{}", std::str::from_utf8(&rsp_buf[..read]).unwrap());

    Ok(())
}
