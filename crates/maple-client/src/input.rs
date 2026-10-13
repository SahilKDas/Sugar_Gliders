use maple_protocol::Command;
#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    League(usize),
    Channel(usize),
    Group(usize),
    Voice,
    Composer,
    Send,
    About,
    None,
}
pub fn hit_test(x: f64, y: f64, width: f64, height: f64, groups: usize) -> Control {
    if (11.0..57.0).contains(&x) && (22.0..69.0).contains(&y) {
        return Control::About;
    }
    if (14.0..54.0).contains(&x) && (90.0..130.0).contains(&y) {
        return Control::League(0);
    }
    if (14.0..54.0).contains(&x) && (134.0..174.0).contains(&y) {
        return Control::League(1);
    }
    if (78.0..252.0).contains(&x) && (190.0..322.0).contains(&y) {
        let index = ((y - 190.0) / 44.0) as usize;
        if (y - 190.0) % 44.0 < 38.0 {
            return Control::Channel(index);
        }
    }
    if (78.0..252.0).contains(&x) && (382.0..426.0).contains(&y) {
        return Control::Voice;
    }
    let visible = ((height - 560.0) / 36.0).max(0.0) as usize;
    if (78.0..252.0).contains(&x) && y >= 470.0 && y < 470.0 + visible.min(groups) as f64 * 36.0 {
        return Control::Group(((y - 470.0) / 36.0) as usize);
    }
    if (280.0..width - 20.0).contains(&x) && (height - 91.0..height - 43.0).contains(&y) {
        return if x >= width - 106.0 {
            Control::Send
        } else {
            Control::Composer
        };
    }
    Control::None
}
pub fn parse(text: &str, room: &str) -> Result<Command, String> {
    if !text.starts_with('/') {
        return Ok(Command::Send {
            channel: room.into(),
            text: text.into(),
        });
    }
    let parts: Vec<_> = text.split_whitespace().collect();
    match parts.as_slice(){
        ["/logout"]=>Ok(Command::Logout),
        ["/join",channel]=>Ok(Command::Join{channel:(*channel).into()}),
        ["/dm",id,member]=>Ok(Command::CreateGroup{id:(*id).into(),members:vec![(*member).into()],persistent:true}),
        [verb,id,members] if *verb=="/group"||*verb=="/ephemeral"=>Ok(Command::CreateGroup{id:(*id).into(),members:members.split(',').map(str::to_owned).collect(),persistent:*verb=="/group"}),
        ["/invite",member]=>Ok(Command::Invite{channel:room.into(),member:(*member).into()}),
        ["/leave"]=>Ok(Command::Leave{channel:room.into()}),
        ["/close"]=>Ok(Command::Close{channel:room.into()}),
        _=>Err("Commands: /dm id user · /group id user,user · /ephemeral id user · /join id · /invite user · /leave · /close".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_match_visible_layout() {
        assert_eq!(hit_test(34.0, 110.0, 1100.0, 760.0, 2), Control::League(0));
        assert_eq!(hit_test(34.0, 154.0, 1100.0, 760.0, 2), Control::League(1));
        assert_eq!(
            hit_test(100.0, 254.0, 1100.0, 760.0, 2),
            Control::Channel(1)
        );
        assert_eq!(hit_test(100.0, 232.0, 1100.0, 760.0, 2), Control::None);
        assert_eq!(hit_test(400.0, 690.0, 1100.0, 760.0, 2), Control::Composer);
        assert_eq!(hit_test(1040.0, 690.0, 1100.0, 760.0, 2), Control::Send);
        assert_eq!(hit_test(100.0, 480.0, 760.0, 520.0, 2), Control::None);
    }
}
