use super::{display, Identity, Peer, Shared};
use maple_protocol::{Command, Event, Secret};
use maple_transport::{read, write, Error};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
struct Context {
    user: Option<Identity>,
    token: Option<Secret>,
    room: String,
    last: Instant,
}
fn error(message: impl Into<String>) -> Vec<Event> {
    vec![Event::Error(message.into())]
}
async fn authenticate(
    c: &mut Context,
    shared: &Shared,
    connection: &quinn::Connection,
    id: u32,
    identity: Identity,
    token: Secret,
) -> Vec<Event> {
    shared
        .names
        .write()
        .await
        .insert(identity.id.clone(), identity.username.clone());
    let mut peers = shared.peers.lock().await;
    peers.insert(
        id,
        Peer {
            connection: connection.clone(),
            user: identity.id.clone(),
            room: None,
            session: token.clone(),
        },
    );
    let mut state = shared.state.lock().await;
    state.online.insert(identity.id.clone());
    let snapshot = state.snapshot(&identity.id);
    let _ = shared.bus.send(state.presence(&c.room));
    drop(state);
    drop(peers);
    let username = identity.username.clone();
    c.user = Some(identity);
    c.token = Some(token.clone());
    vec![
        Event::Authenticated { username, token },
        display(snapshot, &shared.names.read().await.clone()),
    ]
}
async fn command(
    c: &mut Context,
    shared: &Shared,
    connection: &quinn::Connection,
    id: u32,
    command: Command,
) -> Vec<Event> {
    match command {
        Command::Register { .. } | Command::Login { .. } => error("Already signed in"),
        Command::Resume { token } if c.user.is_none() => {
            if !shared.auth_allowed(connection.remote_address().ip()).await {
                return error("Sign-in rate limit: try again in a minute");
            }
            match shared.db.resume(token.clone()).await {
                Ok((identity, new)) => {
                    shared.revoke_connections(&token).await;
                    authenticate(c, shared, connection, id, identity, new).await
                }
                Err(_) => vec![Event::SignedOut],
            }
        }
        Command::Logout => {
            if let Some(token) = c.token.take() {
                shared.db.revoke(token).await;
            }
            shared.peers.lock().await.remove(&id);
            vec![Event::SignedOut]
        }
        Command::Ping => vec![Event::Pong],
        command => {
            let Some(identity) = &c.user else {
                return error("Sign in before accessing text or voice");
            };
            if c.last.elapsed() < Duration::from_millis(80) {
                return error("Slow down");
            }
            c.last = Instant::now();
            if !shared.db.valid(c.token.as_ref().unwrap().clone()).await {
                connection.close(0u8.into(), b"Session expired");
                return vec![];
            }
            if let Command::Voice { room } = command {
                if room.is_some_and(|r| !(1..=2).contains(&r)) {
                    return error("Unknown voice room");
                }
                let mut peers = shared.peers.lock().await;
                if room.is_some()
                    && peers.values().filter(|p| p.room == room).count() >= 10
                    && !peers.get(&id).is_some_and(|p| p.room == room)
                {
                    return error("Voice room full");
                }
                if let Some(p) = peers.get_mut(&id) {
                    p.room = room;
                }
                return vec![Event::VoiceJoined(room)];
            }
            let names = shared.names.read().await.clone();
            let resolve = |handle: String| {
                names
                    .iter()
                    .find(|(_, name)| **name == handle)
                    .map(|(id, _)| id.clone())
                    .ok_or("Participant account not found")
            };
            let command = match command {
                Command::CreateGroup {
                    id,
                    members,
                    persistent,
                } => match members
                    .into_iter()
                    .map(resolve)
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(members) => Command::CreateGroup {
                        id,
                        members,
                        persistent,
                    },
                    Err(e) => return error(e),
                },
                Command::Invite { channel, member } => match resolve(member) {
                    Ok(member) => Command::Invite { channel, member },
                    Err(e) => return error(e),
                },
                c => c,
            };
            let mut state = shared.state.lock().await;
            let before = state.clone();
            let previous_room = c.room.clone();
            let replies = match state.apply(&identity.id, &mut c.room, command) {
                Ok(replies) => replies,
                Err(e) => return error(e),
            };
            if let Err(e) = shared.persist(&state).await {
                *state = before;
                c.room = previous_room;
                return error(format!("Storage: {e}"));
            }
            let mut outgoing = Vec::new();
            for e in replies {
                if matches!(e, Event::Message { .. }) {
                    let _ = shared.bus.send(e);
                } else {
                    outgoing.push(display(e, &names));
                }
            }
            let _ = shared.bus.send(state.presence(&c.room));
            outgoing
        }
    }
}
async fn updates(c: &mut Context, shared: &Shared, event: Event) -> Vec<Event> {
    let identity = c.user.as_ref().unwrap();
    let names = shared.names.read().await.clone();
    let state = shared.state.lock().await;
    match event {
        Event::Message { ref channel, .. }
            if channel == &c.room && state.allowed(&identity.id, channel) =>
        {
            vec![display(event, &names)]
        }
        Event::Presence { .. } => {
            let mut events = vec![display(state.snapshot(&identity.id), &names)];
            if !state.allowed(&identity.id, &c.room) {
                c.room = "general".into();
                events.push(Event::Joined(c.room.clone()));
            }
            events.push(display(state.presence(&c.room), &names));
            events
        }
        _ => vec![],
    }
}
pub(super) async fn run(
    connection: quinn::Connection,
    id: u32,
    shared: Arc<Shared>,
) -> Result<(), Error> {
    let (mut output, mut input) =
        tokio::time::timeout(Duration::from_secs(5), connection.accept_bi()).await??;
    let (commands, mut inbox) = tokio::sync::mpsc::channel(16);
    let reader = tokio::spawn(async move {
        loop {
            let Ok(c) = read::<Command>(&mut input, maple_transport::MAX_COMMAND).await else {
                break;
            };
            if commands.send(c).await.is_err() {
                break;
            }
        }
    });
    struct Guard(tokio::task::JoinHandle<()>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _reader = Guard(reader);
    let mut c = Context {
        user: None,
        token: None,
        room: "general".into(),
        last: Instant::now() - Duration::from_secs(1),
    };
    let mut events = shared.bus.subscribe();
    let mut expiry = tokio::time::interval(Duration::from_secs(1));
    let started = Instant::now();
    let mut audio_window = Instant::now();
    let mut audio_count = 0u32;
    loop {
        let outgoing = tokio::select! {
            incoming=inbox.recv()=>{
                let Some(incoming)=incoming else{break;};let register=matches!(&incoming,Command::Register{..});let logout=matches!(&incoming,Command::Logout);
                let outgoing=match incoming{
                    Command::Register{username,password}|Command::Login{username,password} if c.user.is_none()=>{
                        if !shared.auth_allowed(connection.remote_address().ip()).await{error("Sign-in rate limit: try again in a minute")}else{match shared.db.password(username,password,register).await{Ok((identity,token))=>authenticate(&mut c,&shared,&connection,id,identity,token).await,Err(e)=>error(e)}}
                    },other=>command(&mut c,&shared,&connection,id,other).await
                };
                if logout{for e in outgoing{write(&mut output,&e).await?;}break;}outgoing
            },
            incoming=events.recv(),if c.user.is_some()=>match incoming{Ok(e)=>updates(&mut c,&shared,e).await,Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>error("History changed while disconnected; rejoin the channel"),Err(_)=>break},
            bytes=connection.read_datagram()=>{
                let bytes=bytes?;if audio_window.elapsed()>=Duration::from_secs(1){audio_window=Instant::now();audio_count=0;}audio_count+=1;if audio_count>75{continue;}
                let Some((room,_,seq,payload))=maple_audio::unpack(&bytes) else{continue;};let peers=shared.peers.lock().await;let Some(sender)=peers.get(&id) else{continue;};if sender.room!=Some(room){continue;}
                let packet=maple_audio::packet(room,id,seq,payload);for (_,p) in peers.iter().filter(|(target,p)|**target!=id&&p.room==Some(room)){if p.connection.max_datagram_size().is_some_and(|max|packet.len()<=max){let _=p.connection.send_datagram(packet.clone().into());}}continue;
            },
            _=expiry.tick()=>{if let Some(t)=&c.token{if !shared.db.valid(t.clone()).await{break;}}else if started.elapsed()>Duration::from_secs(300){break;}continue;}
        };
        for event in outgoing {
            write(&mut output, &event).await?;
        }
    }
    Ok(())
}
