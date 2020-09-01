#![type_length_limit = "1406993"]
use std::sync::{atomic::AtomicBool, atomic::Ordering::Relaxed, Arc};

use bytes::Bytes;
use bytestring::ByteString;
use futures::{future::ok, SinkExt, StreamExt};
use ntex::server;
use ntex_codec::Framed;

use ntex_mqtt::v3::{client, codec, Connect, ConnectAck, ControlMessage, MqttServer};

struct St;

async fn connect<Io>(mut packet: Connect<Io>) -> Result<ConnectAck<Io, St>, ()> {
    println!("CONNECT: {:?}", packet);
    packet.packet();
    packet.packet_mut();
    packet.io();
    packet.sink();
    Ok(packet.ack(St, false).idle_timeout(16))
}

#[ntex::test]
async fn test_simple() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "ntex_mqtt=trace,ntex_codec=info,ntex=trace");
    env_logger::init();

    let srv = server::test_server(|| MqttServer::new(connect).publish(|_t| ok(())).finish());

    // connect to server
    let client =
        client::MqttConnector::new(srv.addr()).client_id("user").connect().await.unwrap();

    let sink = client.sink();

    ntex::rt::spawn(client.start_default());

    let res =
        sink.publish(ByteString::from_static("#"), Bytes::new()).send_at_least_once().await;
    assert!(res.is_ok());

    sink.close();
    Ok(())
}

#[ntex::test]
async fn test_connect_fail() -> std::io::Result<()> {
    // bad user name or password
    let srv = server::test_server(|| {
        MqttServer::new(|conn: Connect<_>| ok::<_, ()>(conn.bad_username_or_pwd::<St>()))
            .publish(|_t| ok(()))
            .finish()
    });
    let err =
        client::MqttConnector::new(srv.addr()).client_id("user").connect().await.err().unwrap();
    if let client::ClientError::Ack { session_present, return_code } = err {
        assert!(!session_present);
        assert_eq!(return_code, codec::ConnectAckReason::BadUserNameOrPassword);
    }

    // identifier rejected
    let srv = server::test_server(|| {
        MqttServer::new(|conn: Connect<_>| ok::<_, ()>(conn.identifier_rejected::<St>()))
            .publish(|_t| ok(()))
            .finish()
    });
    let err =
        client::MqttConnector::new(srv.addr()).client_id("user").connect().await.err().unwrap();
    if let client::ClientError::Ack { session_present, return_code } = err {
        assert!(!session_present);
        assert_eq!(return_code, codec::ConnectAckReason::IdentifierRejected);
    }

    // not authorized
    let srv = server::test_server(|| {
        MqttServer::new(|conn: Connect<_>| ok::<_, ()>(conn.not_authorized::<St>()))
            .publish(|_t| ok(()))
            .finish()
    });
    let err =
        client::MqttConnector::new(srv.addr()).client_id("user").connect().await.err().unwrap();
    if let client::ClientError::Ack { session_present, return_code } = err {
        assert!(!session_present);
        assert_eq!(return_code, codec::ConnectAckReason::NotAuthorized);
    }

    // service unavailable
    let srv = server::test_server(|| {
        MqttServer::new(|conn: Connect<_>| ok::<_, ()>(conn.service_unavailable::<St>()))
            .publish(|_t| ok(()))
            .finish()
    });
    let err =
        client::MqttConnector::new(srv.addr()).client_id("user").connect().await.err().unwrap();
    if let client::ClientError::Ack { session_present, return_code } = err {
        assert!(!session_present);
        assert_eq!(return_code, codec::ConnectAckReason::ServiceUnavailable);
    }

    Ok(())
}

#[ntex::test]
async fn test_ping() -> std::io::Result<()> {
    let ping = Arc::new(AtomicBool::new(false));
    let ping2 = ping.clone();

    let srv = server::test_server(move || {
        let ping = ping2.clone();
        MqttServer::new(connect)
            .publish(|_| ok(()))
            .control(move |msg| {
                let ping = ping.clone();
                match msg {
                    ControlMessage::Ping(msg) => {
                        ping.store(true, Relaxed);
                        ok(msg.ack())
                    }
                    _ => ok(msg.disconnect()),
                }
            })
            .finish()
    });

    let io = srv.connect().unwrap();
    let mut framed = Framed::new(io, codec::Codec::default());
    framed
        .send(codec::Packet::Connect(codec::Connect::default().client_id("user")))
        .await
        .unwrap();
    let _ = framed.next().await.unwrap().unwrap();

    framed.send(codec::Packet::PingRequest).await.unwrap();
    let pkt = framed.next().await.unwrap().unwrap();
    assert_eq!(pkt, codec::Packet::PingResponse);
    assert!(ping.load(Relaxed));

    Ok(())
}
