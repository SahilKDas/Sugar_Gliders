use maple_protocol::{Command, Event, Secret};
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, watch};
pub enum ClientEvent {
    Wire(Event),
    Connection(bool, String),
    Ready(quinn::Connection),
}
pub async fn run(
    directory: PathBuf,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ClientEvent>,
    room: watch::Receiver<String>,
) {
    let mut remembered: Option<Secret> = None;
    let mut trust: Option<Vec<u8>> = None;
    while !events.is_closed() {
        if trust.is_none() {
            trust = std::fs::read(directory.join("server.der")).ok();
        }
        let Some(cert) = trust.clone() else {
            let _ = events
                .send(ClientEvent::Connection(
                    false,
                    "Waiting for local server certificate…".into(),
                ))
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let result = async {
            let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
            endpoint.set_default_client_config(maple_transport::client_config(cert)?);
            let connection = tokio::time::timeout(
                Duration::from_secs(5),
                endpoint.connect("127.0.0.1:8787".parse()?, "localhost")?,
            )
            .await??;
            let (mut output, input) = connection.open_bi().await?;
            let initial = remembered
                .as_ref()
                .map(|t| Command::Resume { token: t.clone() })
                .unwrap_or(Command::Ping);
            maple_transport::write(&mut output, &initial).await?;
            Ok::<_, maple_transport::Error>((endpoint, connection, output, input))
        }
        .await;
        let (endpoint, connection, mut output, mut input) = match result {
            Ok(v) => v,
            Err(_) => {
                let _ = events
                    .send(ClientEvent::Connection(
                        false,
                        "Secure connection failed • certificate verification stays enabled".into(),
                    ))
                    .await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let _ = events
            .send(ClientEvent::Connection(
                true,
                "Encrypted TLS 1.3 connection • sign in to chat".into(),
            ))
            .await;
        let _ = events.send(ClientEvent::Ready(connection.clone())).await;
        let (incoming, mut messages) = mpsc::channel(32);
        let reader = tokio::spawn(async move {
            loop {
                let Ok(e) =
                    maple_transport::read::<Event>(&mut input, maple_transport::MAX_FRAME).await
                else {
                    break;
                };
                if incoming.send(e).await.is_err() {
                    break;
                }
            }
        });
        let mut last = tokio::time::Instant::now() - Duration::from_secs(1);
        let mut logged_out = false;
        loop {
            tokio::select! {
                command=commands.recv()=>{let Some(c)=command else{reader.abort();return;};if matches!(&c,Command::Logout){remembered=None;logged_out=true;let _=events.send(ClientEvent::Wire(Event::SignedOut)).await;}tokio::time::sleep_until(last+Duration::from_millis(100)).await;if maple_transport::write(&mut output,&c).await.is_err(){break;}last=tokio::time::Instant::now();},
                event=messages.recv()=>{let Some(e)=event else{break;};match &e{
                    Event::Authenticated{token,..}=>{remembered=Some(token.clone());let channel=room.borrow().clone();tokio::time::sleep(Duration::from_millis(100)).await;let _=maple_transport::write(&mut output,&Command::Join{channel}).await;last=tokio::time::Instant::now();},
                    Event::SignedOut=>remembered=None,_=>{}
                }if events.send(ClientEvent::Wire(e)).await.is_err(){reader.abort();return;}},
                _=connection.closed()=>break
            }
        }
        reader.abort();
        connection.close(0u8.into(), b"Reconnect");
        drop(endpoint);
        while commands.try_recv().is_ok() {}
        let _ = events
            .send(ClientEvent::Connection(
                false,
                if logged_out {
                    "Signed out • reconnecting securely"
                } else {
                    "Disconnected • resuming encrypted session"
                }
                .into(),
            ))
            .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
