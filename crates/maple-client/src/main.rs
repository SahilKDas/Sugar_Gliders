mod input;
mod ui;
use futures_util::{SinkExt, StreamExt};
use maple_protocol::{Command, Event};
use std::{collections::VecDeque, num::NonZeroU32, rc::Rc, sync::mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};
struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    ui: ui::Painter,
    draft: String,
    room: String,
    groups: Vec<maple_protocol::Group>,
    members: Vec<String>,
    scroll: usize,
    cursor: (f64, f64),
    messages: VecDeque<(String, String, String)>,
    status: String,
    commands: tokio::sync::mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    voice: Option<maple_audio::VoiceSession>,
}
const ROOMS: [&str; 3] = ["general", "introductions", "campfire"];
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match el.create_window(
            Window::default_attributes()
                .with_title("Maple • Your cozy corner")
                .with_inner_size(winit::dpi::LogicalSize::new(1100, 760))
                .with_min_inner_size(winit::dpi::LogicalSize::new(760, 520)),
        ) {
            Ok(w) => {
                let w = Rc::new(w);
                let context = softbuffer::Context::new(w.clone()).expect("display context");
                self.surface =
                    Some(softbuffer::Surface::new(&context, w.clone()).expect("display surface"));
                self.window = Some(w);
            }
            Err(e) => {
                eprintln!("Window: {e}");
                el.exit();
            }
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Message {
                    channel,
                    author,
                    text,
                } => {
                    self.messages.push_back((channel, author, text));
                    if self.messages.len() > 500 {
                        self.messages.pop_front();
                    }
                }
                Event::Error(s) => self.status = s,
                Event::Groups(groups) => self.groups = groups,
                Event::Presence { channel, members } if channel == self.room => {
                    self.members = members
                }
                Event::Joined(room) => {
                    self.room = room;
                    self.scroll = 0;
                }
                Event::History { channel, messages } => {
                    self.messages.retain(|(c, _, _)| c != &channel);
                    for e in messages {
                        if let Event::Message {
                            channel,
                            author,
                            text,
                        } = e
                        {
                            self.messages.push_back((channel, author, text));
                        }
                    }
                    while self.messages.len() > 500 {
                        self.messages.pop_front();
                    }
                }
                _ => {}
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        el.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::CursorMoved { position, .. } => self.cursor = (position.x, position.y),
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if y > 0.0 {
                    self.scroll = (self.scroll + 1).min(500);
                } else {
                    self.scroll = self.scroll.saturating_sub(1);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if self.cursor.0 < 260.0 && self.cursor.1 >= 190.0 && self.cursor.1 < 322.0 {
                    self.scroll = 0;
                    let _ = self.commands.try_send(Command::Join {
                        channel: ROOMS[((self.cursor.1 - 190.0) / 44.0) as usize].into(),
                    });
                }
                if self.cursor.0 >= 78.0 && self.cursor.0 < 252.0 && self.cursor.1 >= 470.0 {
                    let index = ((self.cursor.1 - 470.0) / 36.0) as usize;
                    if let Some(g) = self.groups.get(index) {
                        let _ = self.commands.try_send(Command::Join {
                            channel: g.id.clone(),
                        });
                    }
                }
                if self.cursor.0 < 260.0 && self.cursor.1 > 380.0 && self.cursor.1 < 430.0 {
                    if self.voice.is_some() {
                        self.voice = None;
                        self.status = "Voice disconnected".into();
                    } else {
                        match maple_audio::VoiceSession::connect("127.0.0.1:8788") {
                            Ok(v) => {
                                self.voice = Some(v);
                                self.status = "Voice connected • local Autumn Lounge".into();
                            }
                            Err(e) => self.status = format!("Voice: {e}"),
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Named(NamedKey::Enter) => {
                        if maple_protocol::valid_text(&self.draft) {
                            let command = match input::parse(&self.draft, &self.room) {
                                Ok(c) => c,
                                Err(e) => {
                                    self.status = e;
                                    return;
                                }
                            };
                            match self.commands.try_send(command) {
                                Ok(()) => {
                                    self.draft.clear();
                                    self.scroll = 0;
                                }
                                Err(_) => {
                                    self.status = "Connection unavailable; draft retained".into()
                                }
                            }
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.draft.pop();
                    }
                    Key::Named(NamedKey::Escape) => self.draft.clear(),
                    Key::Character(s) => {
                        if self.draft.len() + s.len() <= 4096 {
                            self.draft.push_str(&s);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(w) = &self.window else {
                    return;
                };
                let size = w.inner_size();
                let (Some(width), Some(height)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                else {
                    return;
                };
                let speaking = self.voice.as_ref().is_some_and(|v| v.speaking());
                let pixels = self.ui.draw(
                    size.width,
                    size.height,
                    &self.room,
                    &self.messages,
                    &self.draft,
                    &self.status,
                    self.scroll,
                    self.voice.is_some(),
                    speaking,
                    &self.groups,
                    &self.members,
                );
                let surface = self.surface.as_mut().unwrap();
                if surface.resize(width, height).is_ok() {
                    if let Ok(mut buffer) = surface.buffer_mut() {
                        for (dst, src) in buffer.iter_mut().zip(pixels.chunks_exact(4)) {
                            *dst = ((src[0] as u32) << 16) | ((src[1] as u32) << 8) | src[2] as u32;
                        }
                        let _ = buffer.present();
                    }
                }
            }
            _ => {}
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = std::env::args()
        .nth(1)
        .filter(|s| s == "--render")
        .and_then(|_| std::env::args().nth(2))
    {
        let painter = ui::Painter::new()?;
        let mut messages = VecDeque::new();
        messages.push_back((
            "general".into(),
            "Rowan".into(),
            "Welcome to Maple! Pull up a chair by the campfire.".into(),
        ));
        messages.push_back((
            "general".into(),
            "You".into(),
            "Warm colors, native Rust, and a cozy place to chat.".into(),
        ));
        let data = painter.draw(
            1100,
            760,
            "general",
            &messages,
            "",
            "Preview • local development",
            0,
            false,
            false,
            &[],
            &["Rowan".into(), "You".into()],
        );
        tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(1100, 760).unwrap())
            .unwrap()
            .save_png(path)?;
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let (commands, mut receiver) = tokio::sync::mpsc::channel::<Command>(64);
    let (events, inbox) = mpsc::channel();
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Maple guest".into());
    runtime.spawn(async move {
        let result=connect_async("ws://127.0.0.1:8787").await;
        let Ok((ws,_))=result else { let _=events.send(Event::Error("Offline • start maple-server, then restart client".into())); return; };
        let (mut sink,mut stream)=ws.split();
        let _=events.send(Event::Error("Connected • local development server".into()));
        if sink.send(Message::Text(serde_json::to_string(&Command::Hello {name}).unwrap().into())).await.is_err(){return;}
        loop { tokio::select! {
            command=receiver.recv()=>{ let Some(c)=command else{break;}; if sink.send(Message::Text(serde_json::to_string(&c).unwrap().into())).await.is_err(){break;} },
            event=stream.next()=>{ match event { Some(Ok(Message::Text(s)))=>{if let Ok(e)=serde_json::from_str(&s){let _=events.send(e);}}, Some(Ok(_))=>{}, _=>break } }
        }}
        let _=events.send(Event::Error("Disconnected • restart client to reconnect".into()));
    });
    let mut app = App {
        window: None,
        surface: None,
        ui: ui::Painter::new()?,
        draft: String::new(),
        room: "general".into(),
        groups: Vec::new(),
        members: Vec::new(),
        scroll: 0,
        cursor: (0.0, 0.0),
        messages: VecDeque::new(),
        status: "Connecting…".into(),
        commands,
        events: inbox,
        voice: None,
    };
    EventLoop::new()?.run_app(&mut app)?;
    Ok(())
}
