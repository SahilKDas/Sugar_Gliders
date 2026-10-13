use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
};
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Transform};
pub struct Theme;
impl Theme {
    pub const CREAM: u32 = 0xfdf8f0;
    pub const AMBER: u32 = 0xd9742b;
    pub const CRIMSON: u32 = 0x9e2a2b;
    pub const BARK: u32 = 0x2b1e1a;
    pub const GOLD: u32 = 0xd9b875;
}
type GlyphCache = HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)>;
pub struct Painter {
    font: fontdue::Font,
    glyphs: RefCell<GlyphCache>,
}
impl Painter {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let paths = [
            std::env::var("MAPLE_FONT").unwrap_or_default(),
            "C:/Windows/Fonts/segoeui.ttf".into(),
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
            "/System/Library/Fonts/Supplemental/Arial.ttf".into(),
        ];
        for path in paths {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
                {
                    return Ok(Self {
                        font,
                        glyphs: RefCell::new(HashMap::new()),
                    });
                }
            }
        }
        Err("No system font found; set MAPLE_FONT to a TrueType font file".into())
    }
    fn color(c: u32) -> Color {
        Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
    }
    fn rect(p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: u32) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            let mut paint = Paint::default();
            paint.set_color(Self::color(c));
            p.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    fn circle(p: &mut Pixmap, x: f32, y: f32, r: f32, c: u32) {
        let mut paint = Paint::default();
        paint.set_color(Self::color(c));
        if let Some(path) = PathBuilder::from_circle(x, y, r) {
            p.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    fn leaf(p: &mut Pixmap, x: f32, y: f32, c: u32) {
        let mut path = PathBuilder::new();
        path.move_to(x, y - 12.0);
        for (dx, dy) in [
            (4.0, -4.0),
            (11.0, -8.0),
            (8.0, 1.0),
            (14.0, 4.0),
            (3.0, 8.0),
            (0.0, 14.0),
            (-3.0, 8.0),
            (-14.0, 4.0),
            (-8.0, 1.0),
            (-11.0, -8.0),
            (-4.0, -4.0),
        ] {
            path.line_to(x + dx, y + dy);
        }
        path.close();
        if let Some(path) = path.finish() {
            let mut paint = Paint::default();
            paint.set_color(Self::color(c));
            p.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    #[allow(clippy::too_many_arguments)] // Raster primitive: position, style, and clipping width.
    fn text(&self, p: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, c: u32, max: f32) {
        let mut pen = x;
        for ch in s.chars() {
            let mut cache = self.glyphs.borrow_mut();
            if cache.len() > 2048 {
                cache.clear();
            }
            let (m, bitmap) = cache
                .entry((ch, size.to_bits()))
                .or_insert_with(|| self.font.rasterize(ch, size));
            if pen + m.advance_width > x + max {
                break;
            }
            for row in 0..m.height {
                for col in 0..m.width {
                    let px = pen as i32 + m.xmin + col as i32;
                    let py = y as i32 - m.height as i32 - m.ymin + row as i32;
                    if px < 0 || py < 0 || px >= p.width() as i32 || py >= p.height() as i32 {
                        continue;
                    }
                    let alpha = bitmap[row * m.width + col] as u32;
                    let i = (py as usize * p.width() as usize + px as usize) * 4;
                    let data = p.data_mut();
                    for (k, shift) in [16, 8, 0].iter().enumerate() {
                        data[i + k] = ((((c >> shift) & 255) * alpha
                            + data[i + k] as u32 * (255 - alpha))
                            / 255) as u8;
                    }
                }
            }
            pen += m.advance_width;
        }
    }
    fn wrap(&self, s: &str, size: f32, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut width = 0.0;
        for c in s.chars() {
            let advance = self.font.metrics(c, size).advance_width;
            if c == '\n' || width + advance > max {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
            if c != '\n' {
                line.push(c);
                width += advance;
            }
        }
        lines.push(line);
        lines
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        w: u32,
        h: u32,
        room: &str,
        messages: &VecDeque<(String, String, String)>,
        draft: &str,
        status: &str,
        scroll: usize,
        voice: bool,
        speaking: bool,
        groups: &[maple_protocol::Group],
        members: &[String],
        league: usize,
    ) -> Vec<u8> {
        let mut p = Pixmap::new(w, h).expect("window pixmap");
        p.fill(Self::color(Theme::CREAM));
        let wf = w as f32;
        let hf = h as f32;
        Self::rect(&mut p, 0.0, 0.0, 68.0, hf, Theme::BARK);
        Self::circle(&mut p, 34.0, 45.0, 23.0, Theme::AMBER);
        Self::leaf(&mut p, 34.0, 44.0, Theme::CREAM);
        Self::circle(&mut p, 34.0, 110.0, 20.0, Theme::CRIMSON);
        Self::circle(
            &mut p,
            34.0,
            154.0,
            20.0,
            if league == 1 { Theme::AMBER } else { 0x60463b },
        );
        self.text(&mut p, "T", 28.0, 161.0, 18.0, Theme::CREAM, 25.0);
        self.text(&mut p, "A", 28.0, 116.0, 20.0, Theme::CREAM, 30.0);
        Self::rect(&mut p, 68.0, 0.0, 192.0, hf, 0xf3e6d5);
        Self::rect(&mut p, 260.0, 0.0, 1.0, hf, Theme::GOLD);
        self.text(&mut p, "MAPLE", 88.0, 44.0, 24.0, Theme::CRIMSON, 165.0);
        self.text(
            &mut p,
            if league == 0 {
                "Autumn League"
            } else {
                "Trail Friends"
            },
            88.0,
            99.0,
            18.0,
            Theme::BARK,
            165.0,
        );
        self.text(&mut p, "TEXT CHANNELS", 88.0, 164.0, 11.0, 0x806352, 165.0);
        for (i, name) in (if league == 0 {
            ["general", "introductions", "campfire"]
        } else {
            ["trail-chat", "trip-plans", "photos"]
        })
        .iter()
        .enumerate()
        {
            let y = 190.0 + i as f32 * 44.0;
            if *name == room {
                Self::rect(&mut p, 78.0, y, 174.0, 38.0, 0xecc9a6);
            }
            self.text(
                &mut p,
                &format!("# {name}"),
                90.0,
                y + 25.0,
                15.0,
                Theme::BARK,
                159.0,
            );
        }
        self.text(&mut p, "VOICE", 88.0, 366.0, 11.0, 0x806352, 160.0);
        Self::rect(
            &mut p,
            78.0,
            382.0,
            174.0,
            44.0,
            if voice { Theme::CRIMSON } else { Theme::AMBER },
        );
        self.text(
            &mut p,
            if voice { "Leave voice" } else { "Join voice" },
            90.0,
            409.0,
            15.0,
            Theme::CREAM,
            150.0,
        );
        self.text(
            &mut p,
            "LOCAL DEVELOPMENT",
            85.0,
            hf - 48.0,
            10.0,
            0x806352,
            170.0,
        );
        self.text(
            &mut p,
            "DIRECT & GROUP CHATS",
            85.0,
            453.0,
            10.0,
            0x806352,
            170.0,
        );
        for (i, g) in groups
            .iter()
            .take(((hf - 560.0) / 36.0).max(0.0) as usize)
            .enumerate()
        {
            let y = 470.0 + i as f32 * 36.0;
            if g.id == room {
                Self::rect(&mut p, 78.0, y, 174.0, 32.0, 0xecc9a6);
            }
            self.text(
                &mut p,
                &format!("{}  {}/10", g.id, g.members.len()),
                87.0,
                y + 22.0,
                13.0,
                Theme::BARK,
                160.0,
            );
        }
        self.text(
            &mut p,
            "Enter sends · Esc clears",
            85.0,
            hf - 25.0,
            11.0,
            Theme::BARK,
            170.0,
        );
        self.text(
            &mut p,
            &format!("# {room}"),
            284.0,
            43.0,
            23.0,
            Theme::BARK,
            wf - 300.0,
        );
        self.text(
            &mut p,
            "A little warmth. A good conversation.",
            284.0,
            70.0,
            13.0,
            0x806352,
            wf - 310.0,
        );
        Self::rect(&mut p, 260.0, 88.0, wf - 260.0, 1.0, Theme::GOLD);
        let member_x = wf - 155.0;
        let content_width = (member_x - 310.0).max(210.0);
        Self::rect(&mut p, member_x, 89.0, 1.0, hf - 190.0, Theme::GOLD);
        self.text(
            &mut p,
            "YOUR STATUS",
            member_x + 15.0,
            121.0,
            11.0,
            0x806352,
            130.0,
        );
        Self::circle(
            &mut p,
            member_x + 34.0,
            158.0,
            if speaking { 21.0 } else { 18.0 },
            if speaking { Theme::AMBER } else { Theme::GOLD },
        );
        Self::circle(&mut p, member_x + 34.0, 158.0, 15.0, Theme::CRIMSON);
        self.text(
            &mut p,
            "You",
            member_x + 61.0,
            164.0,
            14.0,
            Theme::BARK,
            80.0,
        );
        self.text(
            &mut p,
            if speaking {
                "Speaking"
            } else if voice {
                "Voice on"
            } else {
                "Voice off"
            },
            member_x + 15.0,
            196.0,
            12.0,
            Theme::CRIMSON,
            130.0,
        );
        self.text(
            &mut p,
            &format!("MEMBERS · {}", members.len()),
            member_x + 15.0,
            245.0,
            11.0,
            0x806352,
            130.0,
        );
        for (i, member) in members.iter().take(10).enumerate() {
            let y = 279.0 + i as f32 * 31.0;
            if y > hf - 120.0 {
                break;
            }
            Self::circle(&mut p, member_x + 24.0, y - 4.0, 8.0, Theme::GOLD);
            self.text(&mut p, member, member_x + 42.0, y, 13.0, Theme::BARK, 100.0);
        }
        let mut lines = Vec::new();
        for (channel, author, text) in messages {
            if channel == room {
                lines.push((format!("{author}  •"), true));
                for line in self.wrap(text, 15.0, content_width) {
                    lines.push((line, false));
                }
                lines.push((String::new(), false));
            }
        }
        let capacity = ((hf - 250.0) / 23.0).max(1.0) as usize;
        let end = lines.len().saturating_sub(scroll.min(lines.len()));
        let start = end.saturating_sub(capacity);
        if lines.is_empty() {
            self.text(
                &mut p,
                "Welcome to your cozy corner.",
                288.0,
                150.0,
                22.0,
                Theme::CRIMSON,
                content_width,
            );
            self.text(
                &mut p,
                "Start the server and invite another local client.",
                288.0,
                181.0,
                14.0,
                0x806352,
                content_width,
            );
        }
        for (i, (line, author)) in lines[start..end].iter().enumerate() {
            self.text(
                &mut p,
                line,
                285.0,
                121.0 + i as f32 * 23.0,
                if *author { 13.0 } else { 15.0 },
                if *author { Theme::CRIMSON } else { Theme::BARK },
                content_width,
            );
        }
        Self::rect(&mut p, 280.0, hf - 91.0, wf - 300.0, 48.0, Theme::GOLD);
        Self::rect(&mut p, 281.0, hf - 90.0, wf - 302.0, 46.0, 0xffffff);
        let shown = if draft.is_empty() {
            "Write a message…"
        } else {
            draft
        };
        self.text(
            &mut p,
            shown,
            296.0,
            hf - 61.0,
            15.0,
            Theme::BARK,
            wf - 325.0,
        );
        self.text(&mut p, status, 282.0, hf - 17.0, 11.0, 0x806352, wf - 300.0);
        p.take()
    }
}
