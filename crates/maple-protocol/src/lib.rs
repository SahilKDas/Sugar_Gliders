use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_TEXT: usize = 4096;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group { pub id: String, pub members: BTreeSet<String>, pub persistent: bool }
impl Group {
    pub fn new(id: String, members: impl IntoIterator<Item=String>, persistent: bool) -> Result<Self, String> {
        let members: BTreeSet<_> = members.into_iter().collect();
        if !(2..=10).contains(&members.len()) { return Err("Chats require 2–10 distinct participants".into()); }
        Ok(Self { id, members, persistent })
    }
    pub fn join(&mut self, member: String) -> Result<(), String> {
        if self.members.contains(&member) { return Ok(()); }
        if self.members.len() == 10 { return Err("Chat is full".into()); }
        self.members.insert(member); Ok(())
    }
    pub fn leave(&mut self, member: &str) -> Result<(), String> {
        if self.members.contains(member) && self.members.len() == 2 { return Err("Close the chat instead of leaving fewer than two participants".into()); }
        self.members.remove(member); Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChannelKind { Text, Voice }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel { pub id: String, pub name: String, pub category: String, pub kind: ChannelKind }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct League { pub id: String, pub name: String, pub channels: Vec<Channel> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command { Hello { name: String }, Send { channel: String, text: String }, Join { channel: String } }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Event { Message { channel: String, author: String, text: String }, Presence { channel: String, members: Vec<String> }, Error(String) }
pub fn valid_label(s: &str) -> bool { !s.trim().is_empty() && s.len() <= 64 && !s.chars().any(char::is_control) }
pub fn valid_text(s: &str) -> bool { !s.trim().is_empty() && s.len() <= MAX_TEXT }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn membership_limits() {
        assert!(Group::new("g".into(), ["a".into(), "a".into()], false).is_err());
        let mut g = Group::new("g".into(), ["a".into(), "b".into()], false).unwrap();
        assert!(g.leave("a").is_err());
        for i in 0..8 { g.join(i.to_string()).unwrap(); }
        assert!(g.join("overflow".into()).is_err());
        g.leave("a").unwrap(); assert_eq!(g.members.len(), 9);
    }
}
