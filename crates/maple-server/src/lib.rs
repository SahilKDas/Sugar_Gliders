mod accounts;
mod session;
mod state;
use accounts::{Accounts, Identity};
use maple_protocol::{Event, Secret};
use maple_transport::Error;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex, RwLock, Semaphore};
struct Peer {
    connection: quinn::Connection,
    user: String,
    room: Option<u32>,
    session: Secret,
}
struct Shared {
    state: Mutex<state::State>,
    peers: Mutex<BTreeMap<u32, Peer>>,
    names: RwLock<BTreeMap<String, String>>,
    db: Accounts,
    bus: broadcast::Sender<Event>,
    attempts: Mutex<BTreeMap<IpAddr, (Instant, u32)>>,
}
fn display(event: Event, names: &BTreeMap<String, String>) -> Event {
    let name = |id: String| {
        names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "Unknown account".into())
    };
    match event {
        Event::Message {
            channel,
            author,
            text,
        } => Event::Message {
            channel,
            author: name(author),
            text,
        },
        Event::Groups(mut groups) => {
            for g in &mut groups {
                g.members = std::mem::take(&mut g.members)
                    .into_iter()
                    .map(name)
                    .collect();
            }
            Event::Groups(groups)
        }
        Event::Presence { channel, members } => Event::Presence {
            channel,
            members: members.into_iter().map(name).collect(),
        },
        Event::History { channel, messages } => Event::History {
            channel,
            messages: messages.into_iter().map(|e| display(e, names)).collect(),
        },
        other => other,
    }
}
impl Shared {
    async fn auth_allowed(&self, ip: IpAddr) -> bool {
        let mut attempts = self.attempts.lock().await;
        attempts.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
        if !attempts.contains_key(&ip) && attempts.len() >= 1024 {
            return false;
        }
        let (_, count) = attempts.entry(ip).or_insert((Instant::now(), 0));
        *count += 1;
        *count <= 20
    }
    async fn revoke_connections(&self, token: &Secret) {
        let peers = self.peers.lock().await;
        for p in peers.values().filter(|p| p.session.0 == token.0) {
            p.connection.close(0u8.into(), b"Session revoked");
        }
    }
    async fn persist(&self, state: &state::State) -> Result<(), String> {
        let bytes = state.encode().map_err(|e| e.to_string())?;
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.save_snapshot(bytes).map_err(|e| e.to_string()))
            .await
            .map_err(|_| "Storage unavailable".to_string())?
    }
}
pub async fn run(directory: PathBuf) -> Result<(), Error> {
    let endpoint = start(directory, "127.0.0.1:8787".parse()?).await?;
    std::future::pending::<()>().await;
    drop(endpoint);
    Ok(())
}
/// Separate config for hermetic tests; production local launcher remains loopback-only.
pub async fn start(directory: PathBuf, address: SocketAddr) -> Result<quinn::Endpoint, Error> {
    // Claim the socket before generating persistent identity files; multiple instances cannot race.
    let socket = std::net::UdpSocket::bind(address)?;
    let dir = directory.clone();
    let (db, cert, key, saved, names) = tokio::task::spawn_blocking(move || -> Result<_, Error> {
        let (cert, key) = maple_transport::identity(&dir)?;
        let db = Accounts::open(&dir)?;
        let saved = db.snapshot()?;
        let names = db.names()?;
        Ok((db, cert, key, saved, names))
    })
    .await??;
    let state: state::State = match saved {
        Some(bytes) if bytes.len() <= 32_000_000 => serde_json::from_slice(&bytes)?,
        None => state::State::default(),
        _ => return Err("Snapshot exceeds capacity".into()),
    };
    if state.groups.len() > 64
        || state.history.len() > 70
        || state.groups.values().any(|g| {
            !(2..=10).contains(&g.members.len())
                || g.members.iter().any(|id| !names.contains_key(id))
        })
    {
        return Err("Invalid secure chat snapshot".into());
    }
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(maple_transport::server_config(cert, key)?),
        socket,
        Arc::new(quinn::TokioRuntime),
    )?;
    let (bus, _) = broadcast::channel(256);
    let shared = Arc::new(Shared {
        state: Mutex::new(state),
        peers: Mutex::new(BTreeMap::new()),
        names: RwLock::new(names),
        db,
        bus,
        attempts: Mutex::new(BTreeMap::new()),
    });
    let accepting = endpoint.clone();
    tokio::spawn(async move {
        let permits = Arc::new(Semaphore::new(64));
        let mut next = 1u32;
        while let Some(incoming) = accepting.accept().await {
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                incoming.refuse();
                continue;
            };
            let shared = shared.clone();
            let id = next;
            next = next.wrapping_add(1).max(1);
            tokio::spawn(async move {
                let _permit = permit;
                let Ok(Ok(connection)) =
                    tokio::time::timeout(Duration::from_secs(5), incoming).await
                else {
                    return;
                };
                let _ = session::run(connection.clone(), id, shared.clone()).await;
                let mut peers = shared.peers.lock().await;
                peers.remove(&id);
                let active = peers
                    .values()
                    .map(|p| p.user.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                let mut s = shared.state.lock().await;
                let departed = s.online.difference(&active).cloned().collect::<Vec<_>>();
                for user in departed {
                    s.disconnect(&user);
                }
                let _ = shared.bus.send(s.presence("general"));
                drop(s);
                drop(peers);
                connection.close(0u8.into(), b"Connection ended");
            });
        }
    });
    Ok(endpoint)
}
