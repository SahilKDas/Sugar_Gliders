#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod auth;
mod clipboard;
mod editor;
mod input;
mod network;
mod ui;
use maple_audio::sfx::{Cue, Sounds};
use maple_protocol::{Command, Event};
use network::ClientEvent;
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU32,
    rc::Rc,
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};
struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    ui: ui::Painter,
    draft: String,
    edit: editor::Editor,
    ime_preedit: bool,
    dragging_selection: bool,
    last_typing_sound: std::time::Instant,
    room: String,
    groups: Vec<maple_protocol::Group>,
    members: Vec<String>,
    league: usize,
    last_speaking: bool,
    scroll: usize,
    cursor: (f64, f64),
    messages: VecDeque<(String, String, String)>,
    status: String,
    commands: tokio::sync::mpsc::Sender<Command>,
    events: tokio::sync::mpsc::Receiver<ClientEvent>,
    voice: Option<maple_audio::VoiceSession>,
    connected: bool,
    requested_room: tokio::sync::watch::Sender<String>,
    drafts: HashMap<String, String>,
    composer_focused: bool,
    modifiers: ModifiersState,
    about: bool,
    about_scroll: usize,
    authenticated: bool,
    auth: auth::Form,
    connection: Option<quinn::Connection>,
    runtime: Option<tokio::runtime::Handle>,
    active_user: Option<String>,
    sounds: Option<Sounds>,
    muted: bool,
    last_sound: std::time::Instant,
    started: std::time::Instant,
    last_frame: std::time::Instant,
}
const ROOMS: [&str; 3] = ["general", "introductions", "campfire"];
const TRAIL_ROOMS: [&str; 3] = ["trail-chat", "trip-plans", "photos"];
impl App {
    fn new(
        commands: tokio::sync::mpsc::Sender<Command>,
        events: tokio::sync::mpsc::Receiver<ClientEvent>,
        requested_room: tokio::sync::watch::Sender<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            window: None,
            surface: None,
            ui: ui::Painter::new()?,
            draft: String::new(),
            edit: editor::Editor::default(),
            ime_preedit: false,
            dragging_selection: false,
            last_typing_sound: std::time::Instant::now(),
            room: "general".into(),
            groups: Vec::new(),
            members: Vec::new(),
            league: 0,
            last_speaking: false,
            scroll: 0,
            cursor: (0.0, 0.0),
            messages: VecDeque::new(),
            status: "Starting local backend…".into(),
            commands,
            events,
            voice: None,
            connected: false,
            requested_room,
            drafts: HashMap::new(),
            composer_focused: true,
            modifiers: ModifiersState::empty(),
            about: false,
            about_scroll: 0,
            authenticated: false,
            auth: auth::Form::default(),
            connection: None,
            runtime: None,
            active_user: None,
            sounds: None,
            muted: false,
            last_sound: std::time::Instant::now() - std::time::Duration::from_secs(1),
            started: std::time::Instant::now(),
            last_frame: std::time::Instant::now(),
        })
    }
    fn sound(&mut self, cue: Cue) {
        if self.last_sound.elapsed() >= std::time::Duration::from_millis(75) {
            if let Some(sounds) = &self.sounds {
                sounds.play(cue);
            }
            self.last_sound = std::time::Instant::now();
        }
    }
    fn toggle_sound(&mut self) {
        self.muted = !self.muted;
        self.ui.muted.set(self.muted);
        if let Some(sounds) = &self.sounds {
            sounds.set_muted(self.muted);
        }
        let path = maple_server::data_directory().join("sounds-muted");
        if self.muted {
            let _ = std::fs::write(path, b"muted");
        } else {
            let _ = std::fs::remove_file(path);
        }
        self.sound(Cue::Toggle);
    }
    fn select_room(&mut self, channel: String) {
        if ROOMS.contains(&channel.as_str()) {
            self.league = 0;
        } else if TRAIL_ROOMS.contains(&channel.as_str()) {
            self.league = 1;
        }
        if channel != self.room {
            self.sound(Cue::Navigate);
            self.drafts
                .insert(self.room.clone(), std::mem::take(&mut self.draft));
            self.draft = self.drafts.remove(&channel).unwrap_or_default();
        }
        self.edit.end(&self.draft);
        self.room = channel.clone();
        self.scroll = 0;
        self.composer_focused = true;
        self.members.clear();
        let _ = self.requested_room.send(channel.clone());
        if self.connected && self.authenticated {
            if self.commands.try_send(Command::Join { channel }).is_err() {
                self.status = "Backend busy • channel selected; draft retained".into();
            }
        } else {
            self.status = "Offline • channel selected; you can still write a draft".into();
        }
    }
    fn submit(&mut self) {
        if !maple_protocol::valid_text(&self.draft) {
            return;
        }
        if !self.connected || !self.authenticated {
            self.status = "Offline • reconnecting; your draft is retained".into();
            return;
        }
        let command = match input::parse(&self.draft, &self.room) {
            Ok(c) => c,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if let Command::Join { channel } = command {
            self.draft.clear();
            self.edit.end(&self.draft);
            self.select_room(channel);
            return;
        }
        match self.commands.try_send(command) {
            Ok(()) => {
                self.sound(Cue::Send);
                self.draft.clear();
                self.edit.end(&self.draft);
                self.scroll = 0;
            }
            Err(_) => self.status = "Backend busy • draft retained".into(),
        }
    }
    fn copy_selection(&mut self, cut: bool) {
        #[cfg(windows)]
        if let Some((a, b)) = self.edit.selection() {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let owner = self
                .window
                .as_ref()
                .and_then(|w| w.window_handle().ok())
                .and_then(|h| match h.as_raw() {
                    RawWindowHandle::Win32(w) => Some(w.hwnd.get() as *mut std::ffi::c_void),
                    _ => None,
                });
            if owner.is_some_and(|owner| clipboard::write(&self.draft[a..b], owner)) && cut {
                self.edit.delete(&mut self.draft, true);
            }
        }
        #[cfg(not(windows))]
        let _ = cut;
    }
    fn input_feedback(&mut self) {
        self.ui.elapsed.set(self.started.elapsed().as_secs_f32());
        self.ui
            .force_caret_until
            .set(self.started.elapsed().as_secs_f32() + 0.7);
    }
    fn insert_text(&mut self, text: &str) {
        self.edit
            .insert(&mut self.draft, text, maple_protocol::MAX_TEXT);
        self.input_feedback();
        if self.last_typing_sound.elapsed() >= std::time::Duration::from_millis(75) {
            if let Some(sounds) = &self.sounds {
                sounds.play(Cue::Type);
            }
            self.last_typing_sound = std::time::Instant::now();
        }
    }
    fn sign_in(&mut self) {
        if !self.connected {
            self.status = "Waiting for verified encrypted connection".into();
            return;
        }
        if let Some(c) = self.auth.command() {
            if self.commands.try_send(c).is_err() {
                self.auth.pending = false;
                self.status = "Sign-in unavailable; try again".into();
            } else {
                self.status = "Verifying account…".into();
            }
        }
    }
}
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
                w.set_ime_allowed(true);
                let context = softbuffer::Context::new(w.clone()).expect("display context");
                self.surface =
                    Some(softbuffer::Surface::new(&context, w.clone()).expect("display surface"));
                self.window = Some(w);
                self.window.as_ref().unwrap().request_redraw();
            }
            Err(e) => {
                eprintln!("Window: {e}");
                el.exit();
            }
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            changed = true;
            let event = match event {
                ClientEvent::Wire(e) => e,
                ClientEvent::Ready(c) => {
                    self.connection = Some(c);
                    continue;
                }
                ClientEvent::Connection(connected, status) => {
                    self.connected = connected;
                    self.status = status;
                    if !connected {
                        self.authenticated = false;
                        self.auth.pending = false;
                        self.voice = None;
                        self.connection = None;
                        self.members.clear();
                    }
                    continue;
                }
            };
            match event {
                Event::Authenticated { username, .. } => {
                    if self.active_user.as_ref() != Some(&username) {
                        self.messages.clear();
                        self.groups.clear();
                        self.members.clear();
                        self.drafts.clear();
                        self.draft.clear();
                        self.edit.end(&self.draft);
                    }
                    self.active_user = Some(username.clone());
                    self.sound(Cue::Welcome);
                    self.authenticated = true;
                    self.auth.pending = false;
                    self.auth.username = username;
                    self.status = "Signed in • text and voice use TLS 1.3".into();
                }
                Event::SignedOut => {
                    self.sound(Cue::Goodbye);
                    self.active_user = None;
                    self.authenticated = false;
                    self.voice = None;
                    self.groups.clear();
                    self.members.clear();
                    self.messages.clear();
                    self.drafts.clear();
                    self.draft.clear();
                    self.edit.end(&self.draft);
                    self.auth.pending = false;
                    self.status = "Signed out".into();
                }
                Event::VoiceJoined(Some(room)) => {
                    self.voice = None;
                    if let (Some(c), Some(runtime)) = (&self.connection, &self.runtime) {
                        match maple_audio::VoiceSession::connect(c.clone(), room, runtime) {
                            Ok(v) => {
                                self.voice = Some(v);
                                self.status = "Encrypted voice connected".into();
                                self.sound(Cue::VoiceOn);
                            }
                            Err(e) => {
                                self.status = format!("Audio: {e}");
                                let _ = self.commands.try_send(Command::Voice { room: None });
                            }
                        }
                    }
                }
                Event::VoiceJoined(None) => {
                    self.voice = None;
                    self.status = "Voice disconnected".into();
                    self.sound(Cue::VoiceOff);
                }
                Event::Message {
                    channel,
                    author,
                    text,
                } => {
                    if self.active_user.as_deref() != Some(author.as_str()) {
                        self.sound(Cue::Receive);
                    }
                    self.messages.push_back((channel, author, text));
                    if self.messages.len() > 500 {
                        self.messages.pop_front();
                    }
                }
                Event::Error(s) => {
                    self.sound(Cue::Error);
                    self.status = s;
                    self.auth.pending = false;
                }
                Event::Groups(groups) => {
                    self.groups = groups;
                    if !ROOMS.contains(&self.room.as_str())
                        && !TRAIL_ROOMS.contains(&self.room.as_str())
                        && !self.groups.iter().any(|g| g.id == self.room)
                    {
                        self.select_room("general".into());
                    }
                }
                Event::Presence { channel, members } if channel == self.room => {
                    self.members = members
                }
                Event::Joined(room) => {
                    // Ignore stale acknowledgments for a previous click.
                    if self.requested_room.borrow().as_str() != room {
                        continue;
                    }
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
        let speaking = self.voice.as_ref().is_some_and(|v| v.speaking());
        changed |= speaking != self.last_speaking;
        self.last_speaking = speaking;
        let frame_interval = if speaking { 100 } else { 500 };
        if !self.about
            && self.window.as_ref().is_some_and(|w| w.has_focus())
            && self.last_frame.elapsed() >= std::time::Duration::from_millis(frame_interval)
        {
            self.last_frame = std::time::Instant::now();
            self.ui.elapsed.set(self.started.elapsed().as_secs_f32());
            changed = true;
        }
        if let Some(w) = self.window.as_ref().filter(|_| changed) {
            w.request_redraw();
        }
        el.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        ));
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let redraw = matches!(
            &event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Ime(_)
                | WindowEvent::Focused(_)
        );
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
                self.cursor = (position.x / scale, position.y / scale);
                if let Some(w) = &self.window {
                    let size = w.inner_size().to_logical::<f64>(scale);
                    let control = input::hit_test(
                        self.cursor.0,
                        self.cursor.1,
                        size.width,
                        size.height,
                        self.groups.len(),
                    );
                    w.set_cursor(if control == input::Control::Composer {
                        winit::window::CursorIcon::Text
                    } else if control != input::Control::None || !self.authenticated {
                        winit::window::CursorIcon::Pointer
                    } else {
                        winit::window::CursorIcon::Default
                    });
                    if self.dragging_selection && self.authenticated && !self.about {
                        let caret = self.ui.caret_at(&self.draft, self.cursor.0 as f32 - 296.0);
                        self.edit.move_to(&self.draft, caret, true);
                    }
                    self.ui.cursor.set(self.cursor);
                    w.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.dragging_selection = false,
            WindowEvent::Focused(false) => self.dragging_selection = false,
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Ime(Ime::Preedit(text, _)) => {
                self.ime_preedit = !text.is_empty();
            }
            WindowEvent::Ime(Ime::Disabled) => self.ime_preedit = false,
            WindowEvent::Ime(Ime::Commit(text)) if !self.about => {
                self.ime_preedit = false;
                self.input_feedback();
                if self.authenticated {
                    if self.composer_focused {
                        self.insert_text(&text)
                    }
                } else {
                    self.auth.insert(&text)
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if self.about {
                    if y < 0.0 {
                        self.about_scroll = self.about_scroll.saturating_add(3).min(500);
                    } else {
                        self.about_scroll = self.about_scroll.saturating_sub(3);
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
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
                let Some(w) = &self.window else {
                    return;
                };
                let size = w.inner_size().to_logical::<f64>(w.scale_factor());
                let control = input::hit_test(
                    self.cursor.0,
                    self.cursor.1,
                    size.width,
                    size.height,
                    self.groups.len(),
                );
                if control == input::Control::Sound {
                    self.toggle_sound();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                if self.about {
                    if control == input::Control::About {
                        self.about = false;
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                if !self.authenticated && control != input::Control::About {
                    if self
                        .auth
                        .click(self.cursor.0, self.cursor.1, size.width, size.height)
                    {
                        self.sign_in();
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                match control {
                    input::Control::Sound => self.toggle_sound(),
                    input::Control::About => {
                        self.sound(Cue::Navigate);
                        self.about = true;
                    }
                    input::Control::Logout => {
                        self.voice = None;
                        let _ = self.commands.try_send(Command::Logout);
                    }
                    input::Control::League(index) => {
                        self.league = index;
                        let channels = if index == 0 { ROOMS } else { TRAIL_ROOMS };
                        self.select_room(channels[0].into());
                    }
                    input::Control::Channel(index) => self.select_room(
                        (if self.league == 0 { ROOMS } else { TRAIL_ROOMS })[index].into(),
                    ),
                    input::Control::Group(index) => {
                        if let Some(g) = self.groups.get(index) {
                            self.select_room(g.id.clone());
                        }
                    }
                    input::Control::Composer => {
                        self.composer_focused = true;
                        self.dragging_selection = true;
                        let caret = self.ui.caret_at(&self.draft, self.cursor.0 as f32 - 296.0);
                        self.edit
                            .move_to(&self.draft, caret, self.modifiers.shift_key());
                        self.input_feedback();
                    }
                    input::Control::Send => self.submit(),
                    input::Control::None => self.composer_focused = false,
                    input::Control::Voice => {
                        if !self.connected {
                            self.status = "Voice requires a connection to the local backend".into();
                        } else {
                            let room = if self.voice.is_some() {
                                None
                            } else {
                                Some(self.league as u32 + 1)
                            };
                            let _ = self.commands.try_send(Command::Voice { room });
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.input_feedback();
                if self.ime_preedit {
                    return;
                }
                if event.logical_key == Key::Named(NamedKey::F3) {
                    self.toggle_sound();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                if !self.authenticated
                    && !self.about
                    && event.logical_key != Key::Named(NamedKey::F1)
                {
                    match event.logical_key {
                        Key::Character(ref s)
                            if self.modifiers.control_key() && s.eq_ignore_ascii_case("v") =>
                        {
                            if let Some(text) = clipboard::read() {
                                self.auth.insert(&text);
                            }
                        }
                        Key::Named(NamedKey::Tab) => self.auth.field = 1 - self.auth.field,
                        Key::Named(NamedKey::Enter) => self.sign_in(),
                        Key::Named(NamedKey::ArrowLeft) => self.auth.navigate(
                            false,
                            self.modifiers.shift_key(),
                            self.modifiers.control_key(),
                        ),
                        Key::Named(NamedKey::ArrowRight) => self.auth.navigate(
                            true,
                            self.modifiers.shift_key(),
                            self.modifiers.control_key(),
                        ),
                        Key::Named(NamedKey::Home) => {
                            self.auth.edge(false, self.modifiers.shift_key())
                        }
                        Key::Named(NamedKey::End) => {
                            self.auth.edge(true, self.modifiers.shift_key())
                        }
                        Key::Named(NamedKey::Delete) => self.auth.delete(false),
                        Key::Character(ref s)
                            if self.modifiers.control_key() && s.eq_ignore_ascii_case("a") =>
                        {
                            self.auth.select_all()
                        }
                        Key::Named(NamedKey::Backspace) => self.auth.backspace(),
                        Key::Character(s)
                            if !self.ime_preedit
                                && !self.modifiers.control_key()
                                && !self.modifiers.super_key() =>
                        {
                            self.auth.insert(event.text.as_deref().unwrap_or(&s))
                        }
                        _ => {}
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                match event.logical_key {
                    Key::Named(NamedKey::F2) if self.authenticated => {
                        self.voice = None;
                        let _ = self.commands.try_send(Command::Logout);
                    }
                    Key::Named(NamedKey::F1) => {
                        self.about = !self.about;
                        self.about_scroll = 0;
                    }
                    Key::Named(NamedKey::Escape) if self.about => self.about = false,
                    _ if self.about => {}
                    Key::Named(NamedKey::Tab) => self.composer_focused = true,
                    Key::Named(NamedKey::Enter) => {
                        self.submit();
                    }
                    Key::Named(NamedKey::ArrowLeft) if self.composer_focused => {
                        self.edit.horizontal(
                            &self.draft,
                            false,
                            self.modifiers.shift_key(),
                            self.modifiers.control_key(),
                        )
                    }
                    Key::Named(NamedKey::ArrowRight) if self.composer_focused => {
                        self.edit.horizontal(
                            &self.draft,
                            true,
                            self.modifiers.shift_key(),
                            self.modifiers.control_key(),
                        )
                    }
                    Key::Named(NamedKey::Home) if self.composer_focused => {
                        self.edit
                            .move_to(&self.draft, 0, self.modifiers.shift_key())
                    }
                    Key::Named(NamedKey::End) if self.composer_focused => {
                        self.edit
                            .move_to(&self.draft, self.draft.len(), self.modifiers.shift_key())
                    }
                    Key::Named(NamedKey::Delete) if self.composer_focused => {
                        self.edit.delete(&mut self.draft, false)
                    }
                    Key::Character(ref s)
                        if self.composer_focused
                            && self.modifiers.control_key()
                            && s.eq_ignore_ascii_case("v") =>
                    {
                        if let Some(text) = clipboard::read() {
                            self.insert_text(&text);
                        }
                    }
                    Key::Character(ref s)
                        if self.composer_focused
                            && self.modifiers.control_key()
                            && s.eq_ignore_ascii_case("c") =>
                    {
                        self.copy_selection(false)
                    }
                    Key::Character(ref s)
                        if self.composer_focused
                            && self.modifiers.control_key()
                            && s.eq_ignore_ascii_case("x") =>
                    {
                        self.copy_selection(true)
                    }
                    Key::Character(ref s)
                        if self.composer_focused
                            && self.modifiers.control_key()
                            && s.eq_ignore_ascii_case("a") =>
                    {
                        self.edit.select_all(&self.draft)
                    }
                    Key::Named(NamedKey::Backspace) if self.composer_focused => {
                        self.edit.delete(&mut self.draft, true);
                    }
                    Key::Named(NamedKey::Escape) => {
                        self.draft.clear();
                        self.edit.end(&self.draft);
                    }
                    Key::Character(s)
                        if self.composer_focused
                            && !self.ime_preedit
                            && !self.modifiers.control_key()
                            && !self.modifiers.super_key() =>
                    {
                        self.insert_text(event.text.as_deref().unwrap_or(&s));
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
                let logical = size.to_logical::<u32>(w.scale_factor());
                self.ui.caret.set(self.edit.caret);
                self.ui.selection.set(self.edit.selection());
                self.ui.scale.set(w.scale_factor() as f32);
                self.ui.pixel_size.set(Some((size.width, size.height)));
                let mut pixels = self.ui.draw(
                    logical.width,
                    logical.height,
                    &self.room,
                    &self.messages,
                    &self.draft,
                    &self.status,
                    self.scroll,
                    self.voice.is_some(),
                    speaking,
                    &self.groups,
                    &self.members,
                    self.league,
                    self.composer_focused,
                );
                if self.about {
                    pixels =
                        self.ui
                            .about(pixels, logical.width, logical.height, self.about_scroll);
                } else if !self.authenticated {
                    pixels = self.ui.sign_in(
                        pixels,
                        logical.width,
                        logical.height,
                        &self.auth,
                        &self.status,
                    );
                }
                let surface = self.surface.as_mut().unwrap();
                if surface.resize(width, height).is_ok() {
                    if let Ok(mut buffer) = surface.buffer_mut() {
                        for (dst, src) in buffer.iter_mut().zip(pixels.as_chunks::<4>().0) {
                            *dst = ((src[0] as u32) << 16) | ((src[1] as u32) << 8) | src[2] as u32;
                        }
                        let _ = buffer.present();
                    }
                }
            }
            _ => {}
        }
        if redraw {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--licenses") {
        println!(
            "{}\n\n{}",
            include_str!("../../../LICENSE"),
            include_str!("../../../assets/fonts/OFL.txt")
        );
        return Ok(());
    }
    if let Some(path) = std::env::args()
        .nth(1)
        .filter(|s| s == "--render" || s == "--render-login")
        .and_then(|_| std::env::args().nth(2))
    {
        let painter = ui::Painter::new()?;
        let preview_scale = std::env::args()
            .nth(3)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| s.is_finite() && (0.5..=4.0).contains(s))
            .unwrap_or(1.0);
        painter.scale.set(preview_scale);
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
        let mut data = painter.draw(
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
            0,
            true,
        );
        if std::env::args().nth(1).as_deref() == Some("--render-login") {
            data = painter.sign_in(
                data,
                1100,
                760,
                &auth::Form::default(),
                "Sign-in preview • real connections verify the server certificate",
            );
        }
        tiny_skia::Pixmap::from_vec(
            data,
            tiny_skia::IntSize::from_wh(
                (1100.0 * preview_scale).round() as u32,
                (760.0 * preview_scale).round() as u32,
            )
            .unwrap(),
        )
        .unwrap()
        .save_png(path)?;
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let (commands, receiver) = tokio::sync::mpsc::channel::<Command>(64);
    let (events, inbox) = tokio::sync::mpsc::channel(256);
    let (requested_room, room_watch) = tokio::sync::watch::channel("general".to_string());
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("Guest-{}", std::process::id()));
    let data_dir = maple_server::data_directory();
    let network_dir = data_dir.clone();
    let backend_events = events.clone();
    runtime.spawn(async move {
        loop {
            match maple_server::run(data_dir.clone()).await {
                Ok(()) => return,
                Err(e) => {
                    if !e
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::AddrInUse)
                    {
                        let message = format!("Local backend: {e}");
                        let _ = backend_events
                            .send(ClientEvent::Connection(false, message))
                            .await;
                        return;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    runtime.spawn(network::run(network_dir, receiver, events, room_watch));
    let mut app = App::new(commands, inbox, requested_room)?;
    app.muted = maple_server::data_directory().join("sounds-muted").exists();
    app.ui.muted.set(app.muted);
    app.sounds = Some(Sounds::new(app.muted));
    app.runtime = Some(runtime.handle().clone());
    app.auth.username = if name.starts_with("Guest-") {
        String::new()
    } else {
        name.to_ascii_lowercase()
    };
    app.auth.edits[0].end(&app.auth.username);
    EventLoop::new()?.run_app(&mut app)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_navigation_typing_and_drafts() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let (_events, inbox) = tokio::sync::mpsc::channel(8);
        let (selected, room) = tokio::sync::watch::channel("general".into());
        let mut app = App::new(tx, inbox, selected).unwrap();
        app.insert_text("Hello maple 🍁");
        app.select_room("introductions".into());
        assert_eq!(app.room, "introductions");
        assert_eq!(room.borrow().as_str(), "introductions");
        assert!(app.draft.is_empty());
        app.insert_text("My introduction");
        app.submit();
        assert_eq!(app.draft, "My introduction");
        assert!(rx.try_recv().is_err());
        app.select_room("general".into());
        assert_eq!(app.draft, "Hello maple 🍁");
        app.connected = true;
        app.authenticated = true;
        app.submit();
        assert!(
            matches!(rx.try_recv().unwrap(),Command::Send{channel,text} if channel=="general"&&text=="Hello maple 🍁")
        );
        assert!(app.draft.is_empty());
    }
}
