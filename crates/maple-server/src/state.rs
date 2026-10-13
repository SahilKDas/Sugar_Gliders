use maple_protocol::{valid_label, valid_text, Command, Event, Group};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub groups: BTreeMap<String, Group>,
    #[serde(skip)]
    pub online: BTreeSet<String>,
    pub history: BTreeMap<String, VecDeque<Event>>,
    #[serde(skip)]
    pub directory: std::path::PathBuf,
}
pub const CHANNELS: [&str; 6] = [
    "general",
    "introductions",
    "campfire",
    "trail-chat",
    "trip-plans",
    "photos",
];
impl State {
    pub fn load_from(
        directory: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = directory.join("state.json");
        if !path.exists() {
            return Ok(Self {
                directory: directory.to_owned(),
                ..Self::default()
            });
        }
        if path.metadata()?.len() > 64000000 {
            return Err("State file exceeds 64 MB".into());
        }
        let mut state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        state.directory = directory.to_owned();
        if state.groups.len() > 64 || state.history.len() > 70 {
            return Err("Too many saved chats".into());
        }
        for (id, g) in &state.groups {
            if !valid_label(id)
                || id != &g.id
                || !(2..=10).contains(&g.members.len())
                || g.members.iter().any(|m| !valid_label(m))
            {
                return Err("Invalid saved membership".into());
            }
        }
        for (id, history) in &state.history {
            if history.len() > 100
                || (!CHANNELS.contains(&id.as_str()) && !state.groups.contains_key(id))
            {
                return Err("Invalid saved history".into());
            }
            for event in history {
                match event {
                    Event::Message {
                        channel,
                        author,
                        text,
                    } if channel == id && valid_label(author) && valid_text(text) => {}
                    _ => return Err("Invalid saved message".into()),
                }
            }
        }
        Ok(state)
    }
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let groups = self
            .groups
            .iter()
            .filter(|(_, g)| g.persistent)
            .map(|(id, g)| (id.clone(), g.clone()))
            .collect::<BTreeMap<_, _>>();
        let history = self
            .history
            .iter()
            .filter(|(id, _)| CHANNELS.contains(&id.as_str()) || groups.contains_key(*id))
            .map(|(id, h)| (id.clone(), h.clone()))
            .collect();
        let snapshot = Self {
            groups,
            history,
            online: BTreeSet::new(),
            directory: std::path::PathBuf::new(),
        };
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() > 64000000 {
            return Err("Persistent state exceeds 64 MB".into());
        }
        std::fs::create_dir_all(&self.directory)?;
        {
            use std::io::Write;
            let mut file = std::fs::File::create(self.directory.join("state.tmp"))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(
            self.directory.join("state.tmp"),
            self.directory.join("state.json"),
        )?;
        Ok(())
    }
    pub fn allowed(&self, name: &str, channel: &str) -> bool {
        CHANNELS.contains(&channel)
            || self
                .groups
                .get(channel)
                .is_some_and(|g| g.members.contains(name))
    }
    pub fn snapshot(&self, name: &str) -> Event {
        Event::Groups(
            self.groups
                .values()
                .filter(|g| g.members.contains(name))
                .cloned()
                .collect(),
        )
    }
    pub fn presence(&self, channel: &str) -> Event {
        let members = self
            .groups
            .get(channel)
            .map(|g| g.members.iter().cloned().collect())
            .unwrap_or_else(|| self.online.iter().cloned().collect());
        Event::Presence {
            channel: channel.into(),
            members,
        }
    }
    pub fn apply(
        &mut self,
        name: &str,
        room: &mut String,
        command: Command,
    ) -> Result<Vec<Event>, String> {
        if name.is_empty() {
            return Err("Introduce yourself first".into());
        }
        match command {
            Command::Join { channel } => {
                if !self.allowed(name, &channel) {
                    return Err("Unknown channel or membership required".into());
                }
                *room = channel.clone();
                Ok(vec![
                    Event::Joined(channel.clone()),
                    Event::History {
                        channel: channel.clone(),
                        messages: self
                            .history
                            .get(&channel)
                            .map(|h| h.iter().cloned().collect())
                            .unwrap_or_default(),
                    },
                ])
            }
            Command::Send { channel, text } => {
                if channel != *room || !self.allowed(name, &channel) || !valid_text(&text) {
                    return Err("Invalid message or membership".into());
                }
                let event = Event::Message {
                    channel: channel.clone(),
                    author: name.into(),
                    text,
                };
                let history = self.history.entry(channel).or_default();
                history.push_back(event.clone());
                if history.len() > 100 {
                    history.pop_front();
                }
                Ok(vec![event])
            }
            Command::CreateGroup {
                id,
                members,
                persistent,
            } => {
                if self.groups.len() >= 64
                    || !valid_label(&id)
                    || CHANNELS.contains(&id.as_str())
                    || self.groups.contains_key(&id)
                {
                    return Err("Invalid or existing chat name (maximum 64 chats)".into());
                }
                if members.iter().any(|n| !valid_label(n)) {
                    return Err("Invalid participant".into());
                }
                let mut members = members;
                members.push(name.into());
                let group = Group::new(id.clone(), members, persistent)?;
                self.groups.insert(id, group);
                Ok(vec![])
            }
            Command::Invite { channel, member } => {
                if !valid_label(&member) {
                    return Err("Invalid participant".into());
                }
                let g = self.groups.get_mut(&channel).ok_or("Unknown chat")?;
                if !g.members.contains(name) {
                    return Err("Membership required".into());
                }
                g.join(member)?;
                Ok(vec![])
            }
            Command::Leave { channel } => {
                let g = self.groups.get_mut(&channel).ok_or("Unknown chat")?;
                if !g.members.contains(name) {
                    return Err("Membership required".into());
                }
                g.leave(name)?;
                if *room == channel {
                    *room = "general".into();
                }
                Ok(vec![Event::Joined(room.clone())])
            }
            Command::Close { channel } => {
                let g = self.groups.get(&channel).ok_or("Unknown chat")?;
                if !g.members.contains(name) {
                    return Err("Membership required".into());
                }
                self.groups.remove(&channel);
                self.history.remove(&channel);
                if *room == channel {
                    *room = "general".into();
                }
                Ok(vec![Event::Joined(room.clone())])
            }
            Command::Hello { .. } => Err("Already introduced".into()),
        }
    }
    pub fn disconnect(&mut self, name: &str) {
        self.online.remove(name);
        let expired: Vec<_> = self
            .groups
            .iter()
            .filter(|(_, g)| !g.persistent && !g.members.iter().any(|m| self.online.contains(m)))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.groups.remove(&id);
            self.history.remove(&id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privacy_and_lifecycle() {
        let mut s = State::default();
        let mut room = "general".into();
        s.apply(
            "a",
            &mut room,
            Command::CreateGroup {
                id: "dm".into(),
                members: vec!["b".into()],
                persistent: false,
            },
        )
        .unwrap();
        assert!(!s.allowed("c", "dm"));
        assert!(s
            .apply(
                "c",
                &mut room,
                Command::Join {
                    channel: "dm".into()
                }
            )
            .is_err());
        assert!(s
            .apply(
                "a",
                &mut room,
                Command::Leave {
                    channel: "dm".into()
                }
            )
            .is_err());
        s.disconnect("a");
        assert!(s.groups.is_empty());
    }
    #[test]
    fn bounded_history() {
        let mut s = State::default();
        let mut room = "general".into();
        for _ in 0..250 {
            s.apply(
                "a",
                &mut room,
                Command::Send {
                    channel: "general".into(),
                    text: "hello".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(s.history["general"].len(), 100);
    }
}
