use crate::RpcSocket;
use async_std::{net::UdpSocket, task};

#[async_std::test]
async fn echo_succeeds() {
    let server_socket = RpcSocket::bind("0.0.0.0:0")
        .await
        .expect("couldn't bind server");
    let client_socket = UdpSocket::bind("0.0.0.0:0").await.expect(
        "couldn't bind
        client",
    );

    let server_addr = server_socket.local_addr().expect(
        "failed getting server
        socket address",
    );

    let msg: [u8; 6] = [0, 1, 2, 3, 4, 5];
    let mut rsp_msg = msg;

    let server_handle = task::spawn(async move {
        let mut buf = [0u8; 256];
        let (bytes_read, responder) = server_socket
            .recv_from(&mut buf[..])
            .await
            .expect("couldn't read on server socket");

        responder.respond(&buf[..bytes_read]).await
    });

    let written = client_socket
        .send_to(&msg[..], server_addr)
        .await
        .expect("failed sending message");

    server_handle.await.expect("server failure");

    let (read, _) = client_socket
        .recv_from(&mut rsp_msg[..])
        .await
        .expect("failed receiving message");

    assert_eq!(written, read);
    assert_eq!(&msg[0] + 1, rsp_msg[0]);
    assert_eq!(&msg[1..], &rsp_msg[1..]);
}
