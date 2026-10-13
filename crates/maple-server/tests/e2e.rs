use maple_protocol::{Command, Event, Secret};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
struct Client {
    _endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}
impl Client {
    async fn connect(
        address: std::net::SocketAddr,
        cert: Vec<u8>,
        host: &str,
    ) -> Result<Self, maple_transport::Error> {
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
        endpoint.set_default_client_config(maple_transport::client_config(cert)?);
        let connection =
            tokio::time::timeout(Duration::from_secs(3), endpoint.connect(address, host)?)
                .await??;
        let (mut send, recv) = connection.open_bi().await?;
        maple_transport::write(&mut send, &Command::Ping).await?;
        Ok(Self {
            _endpoint: endpoint,
            connection,
            send,
            recv,
        })
    }
    async fn send(&mut self, c: Command) {
        maple_transport::write(&mut self.send, &c).await.unwrap();
        tokio::time::sleep(Duration::from_millis(110)).await;
    }
    async fn until(&mut self, predicate: impl Fn(&Event) -> bool) -> Event {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let e = maple_transport::read(&mut self.recv, maple_transport::MAX_FRAME)
                    .await
                    .unwrap();
                if predicate(&e) {
                    return e;
                }
            }
        })
        .await
        .expect("Expected secure server response")
    }
    async fn register(&mut self, name: &str) -> Secret {
        self.send(Command::Register {
            username: name.into(),
            password: Secret("a sufficiently long test password".into()),
        })
        .await;
        match self
            .until(|e| matches!(e, Event::Authenticated { .. }))
            .await
        {
            Event::Authenticated { token, .. } => token,
            _ => unreachable!(),
        }
    }
}
struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test]
async fn secure_accounts_text_voice_and_certificates() {
    let directory = std::env::current_dir()
        .unwrap()
        .join(format!("../../target/secure-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let _cleanup = Cleanup(directory.clone());
    // Unverified legacy names are never imported into the secure account database.
    std::fs::write(
        directory.join("state.json"),
        b"legacy data must not be loaded",
    )
    .unwrap();
    let server = maple_server::start(directory.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let address = server.local_addr().unwrap();
    let cert = std::fs::read(directory.join("server.der")).unwrap();
    let (untrusted, _) = maple_transport::identity(&directory.join("untrusted")).unwrap();
    assert!(Client::connect(address, untrusted, "localhost")
        .await
        .is_err());
    assert!(Client::connect(address, cert.clone(), "wrong.example")
        .await
        .is_err());
    let mut anonymous = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    anonymous
        .send(Command::Send {
            channel: "general".into(),
            text: "unauthorized".into(),
        })
        .await;
    anonymous
        .until(|e| matches!(e,Event::Error(s) if s.contains("Sign in")))
        .await;
    anonymous.send(Command::Voice { room: Some(1) }).await;
    anonymous
        .until(|e| matches!(e,Event::Error(s) if s.contains("Sign in")))
        .await;
    let mut alice = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    let alice_token = alice.register("alice").await;
    let mut bob = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    let _ = bob.register("bob").await;
    let mut eve = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    let _ = eve.register("eve").await;
    anonymous
        .send(Command::Login {
            username: "alice".into(),
            password: Secret("incorrect test password".into()),
        })
        .await;
    anonymous
        .until(|e| matches!(e,Event::Error(s) if s=="Invalid username or password"))
        .await;
    anonymous
        .send(Command::Register {
            username: "alice".into(),
            password: Secret("another long test password".into()),
        })
        .await;
    anonymous
        .until(|e| matches!(e,Event::Error(s) if s=="Account cannot be registered"))
        .await;
    alice
        .send(Command::CreateGroup {
            id: "private".into(),
            members: vec!["bob".into()],
            persistent: true,
        })
        .await;
    bob.until(|e|matches!(e,Event::Groups(groups) if groups.iter().any(|g|g.id=="private"&&g.members.contains("alice")))).await;
    alice
        .send(Command::Join {
            channel: "private".into(),
        })
        .await;
    bob.send(Command::Join {
        channel: "private".into(),
    })
    .await;
    eve.send(Command::Join {
        channel: "private".into(),
    })
    .await;
    eve.until(|e| matches!(e,Event::Error(s) if s.contains("membership")))
        .await;
    alice
        .send(Command::Send {
            channel: "private".into(),
            text: "Encrypted private message".into(),
        })
        .await;
    bob.until(|e|matches!(e,Event::Message{author,text,..} if author=="alice"&&text=="Encrypted private message")).await;
    bob.send(Command::Join {
        channel: "private".into(),
    })
    .await;
    bob.until(|e| matches!(e,Event::History{messages,..} if messages.len()==1))
        .await;
    alice.send(Command::Voice { room: Some(1) }).await;
    alice
        .until(|e| matches!(e, Event::VoiceJoined(Some(1))))
        .await;
    bob.send(Command::Voice { room: Some(1) }).await;
    bob.until(|e| matches!(e, Event::VoiceJoined(Some(1))))
        .await;
    anonymous
        .connection
        .send_datagram(maple_audio::packet(1, 999, 1, &[1, 2, 3]).into())
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(180), bob.connection.read_datagram())
            .await
            .is_err()
    );
    alice
        .connection
        .send_datagram(maple_audio::packet(1, 999, 2, &[4, 5, 6]).into())
        .unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(2), bob.connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    let (_, sender, seq, data) = maple_audio::unpack(&bytes).unwrap();
    assert_ne!(sender, 999);
    assert_eq!(seq, 2);
    assert_eq!(data, &[4, 5, 6]);
    eve.send(Command::Voice { room: Some(2) }).await;
    eve.until(|e| matches!(e, Event::VoiceJoined(Some(2))))
        .await;
    alice
        .connection
        .send_datagram(maple_audio::packet(1, 0, 3, &[7, 8, 9]).into())
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(180), eve.connection.read_datagram())
            .await
            .is_err()
    );
    bob.send(Command::Voice { room: None }).await;
    bob.until(|e| matches!(e, Event::VoiceJoined(None))).await;
    alice
        .connection
        .send_datagram(maple_audio::packet(1, 0, 4, &[9, 9, 9]).into())
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(180), bob.connection.read_datagram())
            .await
            .is_err()
    );
    // Resume rotates the token and closes the old authenticated connection.
    let mut resumed = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    resumed
        .send(Command::Resume {
            token: alice_token.clone(),
        })
        .await;
    let resumed_token = match resumed
        .until(|e| matches!(e, Event::Authenticated { .. }))
        .await
    {
        Event::Authenticated { token, .. } => token,
        _ => unreachable!(),
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(2), alice.connection.closed())
            .await
            .is_ok()
    );
    anonymous.send(Command::Resume { token: alice_token }).await;
    anonymous.until(|e| matches!(e, Event::SignedOut)).await;
    resumed.send(Command::Logout).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), resumed.connection.closed()).await;
    anonymous
        .send(Command::Resume {
            token: resumed_token,
        })
        .await;
    anonymous.until(|e| matches!(e, Event::SignedOut)).await;
    let c = rusqlite::Connection::open(directory.join("accounts.sqlite")).unwrap();
    let (id, hash): (String, String) = c
        .query_row(
            "SELECT id,hash FROM users WHERE username='alice'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_ne!(id, "alice");
    assert!(hash.starts_with("$argon2id$"));
    let body: Vec<u8> = c
        .query_row("SELECT body FROM snapshots WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert!(!String::from_utf8(body)
        .unwrap()
        .contains("\"author\":\"alice\""));
    drop(c);
    server.close(0u8.into(), b"Restart test");
    server.wait_idle().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let restarted = maple_server::start(directory.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let mut login = Client::connect(restarted.local_addr().unwrap(), cert, "localhost")
        .await
        .unwrap();
    login
        .send(Command::Login {
            username: "bob".into(),
            password: Secret("a sufficiently long test password".into()),
        })
        .await;
    login
        .until(|e| matches!(e,Event::Authenticated{username,..} if username=="bob"))
        .await;
    login
        .send(Command::Join {
            channel: "private".into(),
        })
        .await;
    login
        .until(|e| matches!(e,Event::History{messages,..} if messages.len()==1))
        .await;
    restarted.close(0u8.into(), b"Done");
    restarted.wait_idle().await;
}
#[tokio::test]
async fn wire_capture_and_tamper_rejection() {
    let directory = std::env::current_dir()
        .unwrap()
        .join(format!("../../target/wire-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let _cleanup = Cleanup(directory.clone());
    let server = maple_server::start(directory.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let destination = server.local_addr().unwrap();
    let cert = std::fs::read(directory.join("server.der")).unwrap();
    let proxy = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = proxy.local_addr().unwrap();
    let captures = Arc::new(tokio::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let recorded = captures.clone();
    let corrupt = Arc::new(AtomicBool::new(false));
    let tamper = corrupt.clone();
    let task = tokio::spawn(async move {
        let mut client = None;
        let mut buffer = [0u8; 65536];
        while let Ok((n, from)) = proxy.recv_from(&mut buffer).await {
            let target = if from == destination {
                client
            } else {
                client = Some(from);
                Some(destination)
            };
            if let Some(target) = target {
                recorded.lock().await.push(buffer[..n].to_vec());
                if tamper.load(Ordering::Relaxed) && from != destination && n > 20 {
                    buffer[n - 1] ^= 0xff;
                }
                let _ = proxy.send_to(&buffer[..n], target).await;
            }
        }
    });
    let mut sender = Client::connect(address, cert.clone(), "localhost")
        .await
        .unwrap();
    sender.register("sender").await;
    let mut receiver = Client::connect(destination, cert, "localhost")
        .await
        .unwrap();
    receiver.register("receiver").await;
    sender.send(Command::Voice { room: Some(1) }).await;
    sender
        .until(|e| matches!(e, Event::VoiceJoined(Some(1))))
        .await;
    receiver.send(Command::Voice { room: Some(1) }).await;
    receiver
        .until(|e| matches!(e, Event::VoiceJoined(Some(1))))
        .await;
    let marker = b"unique plaintext opus payload must never occur on wire";
    corrupt.store(true, Ordering::Relaxed);
    sender
        .connection
        .send_datagram(maple_audio::packet(1, 0, 1, marker).into())
        .unwrap();
    assert!(tokio::time::timeout(
        Duration::from_millis(200),
        receiver.connection.read_datagram()
    )
    .await
    .is_err());
    corrupt.store(false, Ordering::Relaxed);
    sender
        .connection
        .send_datagram(maple_audio::packet(1, 0, 2, marker).into())
        .unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(3), receiver.connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(maple_audio::unpack(&bytes).unwrap().3, marker);
    sender
        .send(Command::Send {
            channel: "general".into(),
            text: "unique confidential text marker for wire test".into(),
        })
        .await;
    receiver.until(|e|matches!(e,Event::Message{text,..} if text=="unique confidential text marker for wire test")).await;
    for bytes in captures.lock().await.iter() {
        for secret in [
            marker.as_slice(),
            b"unique confidential text marker for wire test",
            b"a sufficiently long test password".as_slice(),
        ] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret));
        }
    }
    task.abort();
    server.close(0u8.into(), b"Done");
    server.wait_idle().await;
}
