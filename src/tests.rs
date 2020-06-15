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

    let server_handle = task::spawn(async move {
        let mut buf = [0u8; 256];
        let (bytes_read, responder) = server_socket
            .recv(&mut buf[..])
            .await
            .expect("couldn't read on server socket");
        responder
            .respond(&buf[..bytes_read])
            .await
            .expect("failed responding");
    });

    let msg: [u8; 4] = [0, 0, 1, 1];
    let mut rsp_msg = [0u8; 4];

    let written = client_socket
        .send_to(&msg[..], server_addr)
        .await
        .expect("failed sending message");

    server_handle.await;

    let (read, _) = client_socket
        .recv_from(&mut rsp_msg[..])
        .await
        .expect("failed receiving message");

    assert_eq!(written, read);
    assert_eq!(msg[3], rsp_msg[3]);
}
