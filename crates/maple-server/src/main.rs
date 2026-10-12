mod state;
use futures_util::{SinkExt, StreamExt};
use maple_protocol::{valid_label, Command, Event};
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::{broadcast, Mutex, Semaphore},
};
use tokio_tungstenite::{
    accept_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8787").await?;
    let (bus, _) = broadcast::channel::<Event>(256);
    let state = Arc::new(Mutex::new(state::State::default()));
    let permits = Arc::new(Semaphore::new(64));
    let voice = tokio::net::UdpSocket::bind("127.0.0.1:8788").await?;
    tokio::spawn(async move {
        let mut bytes = [0u8; 1282];
        while let Ok((n, peer)) = voice.recv_from(&mut bytes).await {
            if (7..=1281).contains(&n) && &bytes[..4] == b"MPL1" {
                let _ = voice.send_to(&bytes[..n], peer).await;
            }
        }
    });
    println!("Maple local text server: ws://127.0.0.1:8787; microphone loopback: UDP 8788");
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let bus = bus.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let config = WebSocketConfig::default()
                .max_message_size(Some(8192))
                .max_frame_size(Some(8192));
            let Ok(Ok(ws)) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                accept_async_with_config(stream, Some(config)),
            )
            .await
            else {
                return;
            };
            let (mut out, mut input) = ws.split();
            let mut events = bus.subscribe();
            let mut name = String::new();
            let mut room = "general".to_string();
            let mut last_send = tokio::time::Instant::now() - std::time::Duration::from_secs(1);
            loop {
                tokio::select! {
                    incoming = input.next() => {
                        let raw=match incoming {Some(Ok(Message::Text(raw)))=>raw,Some(Ok(Message::Ping(p)))=>{if out.send(Message::Pong(p)).await.is_err(){break;}continue;},Some(Ok(Message::Pong(_)))=>continue,_=>break};
                        let command=serde_json::from_str::<Command>(&raw);
                        let mut replies=Vec::new();let mut publishes=Vec::new();
                        {let mut s=state.lock().await;
                            match command {
                                Ok(Command::Hello{name:n}) if valid_label(&n)&&name.is_empty() => {
                                    if s.online.contains(&n){replies.push(Event::Error("Name already in use".into()));}else{s.online.insert(n.clone());name=n;replies.push(s.snapshot(&name));}
                                },
                                Ok(c) if last_send.elapsed().as_millis()>=100 => {
                                    last_send=tokio::time::Instant::now();
                                    match s.apply(&name,&mut room,c){Ok(result)=>{for e in result{if matches!(e,Event::Message{..}){publishes.push(e);}else{replies.push(e);}}publishes.push(Event::Presence{channel:room.clone(),members:s.online.iter().cloned().collect()});},Err(e)=>replies.push(Event::Error(e))}
                                },
                                _=>replies.push(Event::Error("Invalid command or rate limit exceeded".into()))
                            }
                        }
                        for event in publishes{let _=bus.send(event);}
                        let mut failed=false;for e in replies{if out.send(Message::Text(serde_json::to_string(&e).unwrap().into())).await.is_err(){failed=true;break;}}if failed{break;}
                    },
                    event=events.recv()=>{
                        let e=match event{Ok(e)=>e,Err(broadcast::error::RecvError::Lagged(_))=>Event::Error("Messages were missed; rejoin channel to reload history".into()),Err(_)=>break};
                        let mut deliver=Vec::new();
                        {let s=state.lock().await;match e{
                            Event::Message{ref channel,..} if channel==&room&&s.allowed(&name,channel)=>deliver.push(e),
                            Event::Presence{..}=>{deliver.push(s.snapshot(&name));if !s.allowed(&name,&room){room="general".into();deliver.push(Event::Joined(room.clone()));}},
                            Event::Error(_)=>deliver.push(e),_=>{}
                        }}
                        let mut failed=false;for e in deliver{if out.send(Message::Text(serde_json::to_string(&e).unwrap().into())).await.is_err(){failed=true;break;}}if failed{break;}
                    }
                }
            }
            state.lock().await.disconnect(&name);
            let _ = bus.send(Event::Presence {
                channel: "general".into(),
                members: vec![],
            });
        });
    }
}
