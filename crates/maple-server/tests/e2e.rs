use futures_util::{SinkExt,StreamExt};
use maple_protocol::{Command,Event};
use tokio_tungstenite::{connect_async,tungstenite::Message,MaybeTlsStream,WebSocketStream};
type Socket=WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
struct Server { child:std::process::Child, directory:std::path::PathBuf }
impl Drop for Server {fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();let _=std::fs::remove_dir_all(&self.directory);}}
async fn send(socket:&mut Socket,command:Command){socket.send(Message::Text(serde_json::to_string(&command).unwrap().into())).await.unwrap();tokio::time::sleep(std::time::Duration::from_millis(120)).await;}
async fn until(socket:&mut Socket,predicate:impl Fn(&Event)->bool)->Event {
    tokio::time::timeout(std::time::Duration::from_secs(3),async{loop{let Message::Text(raw)=socket.next().await.unwrap().unwrap() else{continue;};let e:Event=serde_json::from_str(&raw).unwrap();if predicate(&e){return e;}}}).await.expect("Expected server event")
}
#[tokio::test]
async fn messaging_membership_history_and_udp_relay(){
    let directory=std::env::current_dir().unwrap().join(format!("../../target/smoke-{}",std::process::id()));std::fs::create_dir_all(&directory).unwrap();
    let child=std::process::Command::new(env!("CARGO_BIN_EXE_maple-server")).current_dir(&directory).stdout(std::process::Stdio::null()).spawn().unwrap();let mut server=Server{child,directory};
    let mut alice=loop{assert!(server.child.try_wait().unwrap().is_none(),"Server failed to bind; ports 8787/8788 must be free");if let Ok((ws,_))=connect_async("ws://127.0.0.1:8787").await{break ws;}tokio::time::sleep(std::time::Duration::from_millis(50)).await;};
    let (mut bob,_)=connect_async("ws://127.0.0.1:8787").await.unwrap();let (mut eve,_)=connect_async("ws://127.0.0.1:8787").await.unwrap();
    send(&mut alice,Command::Hello{name:"Alice".into()}).await;send(&mut bob,Command::Hello{name:"Bob".into()}).await;send(&mut eve,Command::Hello{name:"Eve".into()}).await;
    send(&mut alice,Command::CreateGroup{id:"private".into(),members:vec!["Bob".into()],persistent:true}).await;
    until(&mut bob,|e|matches!(e,Event::Groups(groups) if groups.iter().any(|g|g.id=="private"))).await;
    send(&mut alice,Command::Join{channel:"private".into()}).await;send(&mut bob,Command::Join{channel:"private".into()}).await;
    send(&mut eve,Command::Join{channel:"private".into()}).await;until(&mut eve,|e|matches!(e,Event::Error(s) if s.contains("membership"))).await;
    send(&mut alice,Command::Send{channel:"private".into(),text:"A warm hello".into()}).await;
    until(&mut bob,|e|matches!(e,Event::Message{channel,author,text} if channel=="private"&&author=="Alice"&&text=="A warm hello")).await;
    send(&mut bob,Command::Join{channel:"private".into()}).await;
    until(&mut bob,|e|matches!(e,Event::History{channel,messages} if channel=="private"&&messages.len()==1)).await;
    send(&mut alice,Command::Leave{channel:"private".into()}).await;until(&mut alice,|e|matches!(e,Event::Error(s) if s.contains("two"))).await;
    assert!(server.directory.join("data/state.json").exists());
    let first=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let second=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let mut bytes=[0;1286];
    first.send_to(&maple_audio::packet(0,1,&[1,2,3]),"127.0.0.1:8788").await.unwrap();tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    second.send_to(&maple_audio::packet(0,2,&[4,5,6]),"127.0.0.1:8788").await.unwrap();
    let (n,_)=tokio::time::timeout(std::time::Duration::from_secs(1),first.recv_from(&mut bytes)).await.unwrap().unwrap();let (id,seq,payload)=maple_audio::unpack(&bytes[..n]).unwrap();assert_eq!(id,second.local_addr().unwrap().port() as u32);assert_eq!(seq,2);assert_eq!(payload,&[4,5,6]);
}
