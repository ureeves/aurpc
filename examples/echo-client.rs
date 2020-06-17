use aurpc::RpcSocket;
use std::env;

#[async_std::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        panic!("needs a socket address as input");
    }

    let buf = b"hello, world";
    let mut rsp_buf = *buf;
    let socket = RpcSocket::bind("0.0.0.0:0").await?;

    let (_, rsp_fut) = socket
        .send_to(&buf[..], &mut rsp_buf[..], args[1].clone())
        .await?;

    let read = rsp_fut.await?;

    println!("{}", std::str::from_utf8(&rsp_buf[..read]).unwrap());

    Ok(())
}
