use maple_protocol::Command;
pub fn parse(text: &str, room: &str) -> Result<Command, String> {
    if !text.starts_with('/') {
        return Ok(Command::Send {
            channel: room.into(),
            text: text.into(),
        });
    }
    let parts: Vec<_> = text.split_whitespace().collect();
    match parts.as_slice(){
        ["/join",channel]=>Ok(Command::Join{channel:(*channel).into()}),
        ["/dm",id,member]=>Ok(Command::CreateGroup{id:(*id).into(),members:vec![(*member).into()],persistent:true}),
        [verb,id,members] if *verb=="/group"||*verb=="/ephemeral"=>Ok(Command::CreateGroup{id:(*id).into(),members:members.split(',').map(str::to_owned).collect(),persistent:*verb=="/group"}),
        ["/invite",member]=>Ok(Command::Invite{channel:room.into(),member:(*member).into()}),
        ["/leave"]=>Ok(Command::Leave{channel:room.into()}),
        ["/close"]=>Ok(Command::Close{channel:room.into()}),
        _=>Err("Commands: /dm id user · /group id user,user · /ephemeral id user · /join id · /invite user · /leave · /close".into())
    }
}
