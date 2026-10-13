use futures_util::{SinkExt, StreamExt};
use maple_protocol::{Command, Event};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message};
pub enum ClientEvent {
    Wire(Event),
    Connection(bool, String),
}
pub async fn run(
    name: String,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ClientEvent>,
    room: watch::Receiver<String>,
) {
    while !events.is_closed() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            connect_async("ws://127.0.0.1:8787"),
        )
        .await;
        let Ok(Ok((ws, _))) = result else {
            let _ = events
                .send(ClientEvent::Connection(
                    false,
                    "Offline • retrying local backend; your draft stays here".into(),
                ))
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        };
        let (mut sink, mut stream) = ws.split();
        if sink
            .send(Message::Text(
                serde_json::to_string(&Command::Hello { name: name.clone() })
                    .unwrap()
                    .into(),
            ))
            .await
            .is_err()
        {
            continue;
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let channel = room.borrow().clone();
        if sink
            .send(Message::Text(
                serde_json::to_string(&Command::Join { channel })
                    .unwrap()
                    .into(),
            ))
            .await
            .is_err()
        {
            continue;
        }
        let _ = events
            .send(ClientEvent::Connection(
                true,
                "Connected • local development session".into(),
            ))
            .await;
        let mut last_write = tokio::time::Instant::now();
        loop {
            tokio::select! {
                command=commands.recv()=>{
                    let Some(c)=command else{return;};
                    tokio::time::sleep_until(last_write+std::time::Duration::from_millis(120)).await;
                    if sink.send(Message::Text(serde_json::to_string(&c).unwrap().into())).await.is_err(){break;}last_write=tokio::time::Instant::now();
                },
                event=stream.next()=>match event{
                    Some(Ok(Message::Text(raw)))=>if let Ok(e)=serde_json::from_str(&raw){if events.send(ClientEvent::Wire(e)).await.is_err(){return;}},
                    Some(Ok(Message::Ping(bytes)))=>{if sink.send(Message::Pong(bytes)).await.is_err(){break;}},
                    Some(Ok(Message::Pong(_)))=>{},_=>break,
                }
            }
        }
        // Never replay potentially non-idempotent commands after a disconnect.
        while commands.try_recv().is_ok() {}
        let _ = events
            .send(ClientEvent::Connection(
                false,
                "Disconnected • reconnecting; your draft stays here".into(),
            ))
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
