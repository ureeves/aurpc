[![Build
Status](https://drone.ureeves.com/api/badges/ureeves/aurpc/status.svg)](https://drone.ureeves.com/ureeves/aurpc)

# aurpc

Asynchronous UDP RPCs.

Exposes a socket-like interface allowing for sending requests and awaiting a
response as well as listening to requests, with UDP as transport.

This is achieved by implementing an 24-bit protocol header on top of UDP
containing 8-bit flags and a 16-bit request id.

```
                    1                   2
0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|     Flags     |           Request Id          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Since a UDP datagram can carry a maximum of 65507 data bytes. This means
that, with the added overhead, each message can be a maximum of `65504
bytes`.

## Examples

```rust
use aurpc::RpcSocket;

let socket = RpcSocket::bind("127.0.0.1:8080").await?;
let mut buf = vec![0u8; 1024];

loop {
    let (n, responder) = socket.recv_from(&mut buf).await?;
    responder.respond(&buf[..n]).await?;
}
```
