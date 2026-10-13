use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
};
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Transform};
use unicode_segmentation::UnicodeSegmentation;
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
    pub cursor: Cell<(f64, f64)>,
    pub muted: Cell<bool>,
    pub elapsed: Cell<f32>,
    pub scale: Cell<f32>,
    pub pixel_size: Cell<Option<(u32, u32)>>,
    pub caret: Cell<usize>,
    pub selection: Cell<Option<(usize, usize)>>,
    pub view_start: Cell<usize>,
    pub force_caret_until: Cell<f32>,
    font: fontdue::Font,
    glyphs: RefCell<GlyphCache>,
}
impl Painter {
    pub fn sign_in(
        &self,
        data: Vec<u8>,
        w: u32,
        h: u32,
        form: &crate::auth::Form,
        status: &str,
    ) -> Vec<u8> {
        let mut p = Pixmap::from_vec(data, self.dimensions(w, h)).unwrap();
        p.fill(Self::color(Theme::CREAM));
        let left = w as f32 / 2.0 - 250.0;
        let top = h as f32 / 2.0 - 200.0;
        for (x, y) in [
            (90.0, 170.0),
            (w as f32 - 85.0, 150.0),
            (w as f32 - 125.0, h as f32 - 135.0),
            (135.0, h as f32 - 110.0),
        ] {
            self.leaf(&mut p, x, y, 0xecd8bd);
        }
        self.rect(&mut p, left - 29.0, top - 32.0, 558.0, 445.0, 0xeadbc8);
        self.rect(&mut p, left - 32.0, top - 36.0, 564.0, 445.0, 0xffffff);
        self.leaf(&mut p, 42.0, 43.0, Theme::AMBER);
        self.text(&mut p, "MAPLE", 65.0, 51.0, 23.0, Theme::CRIMSON, 180.0);
        self.text(
            &mut p,
            if form.register {
                "Create your Maple account"
            } else {
                "Sign in to Maple"
            },
            left,
            top + 22.0,
            25.0,
            Theme::CRIMSON,
            500.0,
        );
        self.text(
            &mut p,
            "Accounts are local to this server. Text and voice use TLS 1.3.",
            left,
            top + 54.0,
            13.0,
            Theme::BARK,
            500.0,
        );
        for (index, y, label) in [(0, top + 90.0, "Username"), (1, top + 160.0, "Password")] {
            self.text(&mut p, label, left, y - 8.0, 13.0, Theme::BARK, 500.0);
            self.rect(
                &mut p,
                left,
                y,
                500.0,
                48.0,
                if form.field == index {
                    Theme::AMBER
                } else {
                    Theme::GOLD
                },
            );
            self.rect(&mut p, left + 1.0, y + 1.0, 498.0, 46.0, 0xffffff);
        }
        self.field(
            &mut p,
            &form.username,
            &form.edits[0],
            left + 14.0,
            top + 121.0,
            470.0,
            form.field == 0 && !form.pending,
            false,
        );
        self.field(
            &mut p,
            &form.password.0,
            &form.edits[1],
            left + 14.0,
            top + 191.0,
            470.0,
            form.field == 1 && !form.pending,
            true,
        );
        self.rect(
            &mut p,
            left,
            top + 235.0,
            500.0,
            45.0,
            if self.hovered(left, top + 235.0, 500.0, 45.0) {
                Theme::CRIMSON
            } else {
                Theme::AMBER
            },
        );
        self.text(
            &mut p,
            if form.pending {
                match (self.elapsed.get() * 3.0) as u32 % 3 {
                    0 => "Verifying.",
                    1 => "Verifying..",
                    _ => "Verifying...",
                }
            } else if form.register {
                "Create account"
            } else {
                "Sign in"
            },
            left + 18.0,
            top + 264.0,
            16.0,
            Theme::CREAM,
            460.0,
        );
        self.text(
            &mut p,
            if form.register {
                "Already have an account? Sign in"
            } else {
                "New here? Create an account"
            },
            left,
            top + 319.0,
            15.0,
            Theme::CRIMSON,
            500.0,
        );
        self.text(
            &mut p,
            "Username: lowercase letters/digits/_ · Password: 12+ characters",
            left,
            top + 351.0,
            12.0,
            Theme::BARK,
            500.0,
        );
        self.text(
            &mut p,
            status,
            left,
            top + 383.0,
            13.0,
            Theme::CRIMSON,
            500.0,
        );
        self.text(
            &mut p,
            "F1: About · F3: sounds · Tab: next field · Enter: submit",
            left,
            h as f32 - 24.0,
            12.0,
            Theme::BARK,
            500.0,
        );
        self.sound_chip(&mut p);
        p.take()
    }
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let font = fontdue::Font::from_bytes(
            &include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf")[..],
            fontdue::FontSettings::default(),
        )?;
        Ok(Self {
            cursor: Cell::new((-1.0, -1.0)),
            muted: Cell::new(false),
            elapsed: Cell::new(0.0),
            scale: Cell::new(1.0),
            pixel_size: Cell::new(None),
            caret: Cell::new(usize::MAX),
            selection: Cell::new(None),
            view_start: Cell::new(0),
            force_caret_until: Cell::new(0.0),
            font,
            glyphs: RefCell::new(HashMap::new()),
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn field(
        &self,
        p: &mut Pixmap,
        value: &str,
        edit: &crate::editor::Editor,
        x: f32,
        y: f32,
        max: f32,
        active: bool,
        masked: bool,
    ) {
        let shown = if masked {
            "•".repeat(value.graphemes(true).count())
        } else {
            value.to_string()
        };
        let map = |i: usize| {
            if masked {
                value[..i.min(value.len())].graphemes(true).count() * "•".len()
            } else {
                i.min(value.len())
            }
        };
        let caret = map(edit.caret);
        let mut start = 0;
        while self.measure(&shown[start..caret], 16.0) > max - 4.0 {
            start += shown[start..].chars().next().map_or(0, char::len_utf8);
        }
        let mut end = start;
        for (i, c) in shown[start..].char_indices() {
            let next = start + i + c.len_utf8();
            if self.measure(&shown[start..next], 16.0) > max {
                break;
            }
            end = next;
        }
        if let Some((a, b)) = edit.selection().filter(|_| active) {
            let a = map(a).max(start).min(end);
            let b = map(b).max(start).min(end);
            if a < b {
                let left = self.measure(&shown[start..a], 16.0);
                self.rect(
                    p,
                    x + left,
                    y - 18.0,
                    self.measure(&shown[start..b], 16.0) - left,
                    24.0,
                    0xf2d1ac,
                );
            }
        }
        self.text(p, &shown[start..end], x, y, 16.0, Theme::BARK, max);
        if active && self.caret_visible() {
            self.rect(
                p,
                x + self.measure(&shown[start..caret], 16.0),
                y - 18.0,
                1.2,
                23.0,
                Theme::AMBER,
            );
        }
    }
    fn dimensions(&self, w: u32, h: u32) -> tiny_skia::IntSize {
        let (w, h) = self.pixel_size.get().unwrap_or((
            (w as f32 * self.scale.get()).round() as u32,
            (h as f32 * self.scale.get()).round() as u32,
        ));
        tiny_skia::IntSize::from_wh(w, h).unwrap()
    }
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        let scale = self.scale.get();
        let mut previous = None;
        let mut width = 0.0;
        for ch in text.chars() {
            width += previous
                .and_then(|p| self.font.horizontal_kern(p, ch, size * scale))
                .unwrap_or(0.0);
            width += self.font.metrics(ch, size * scale).advance_width;
            previous = Some(ch);
        }
        width / scale
    }
    pub fn caret_at(&self, draft: &str, x: f32) -> usize {
        let start = self.view_start.get().min(draft.len());
        let mut previous_width = 0.0;
        for (offset, ch) in draft[start..].grapheme_indices(true) {
            let end = start + offset + ch.len();
            let width = self.measure(&draft[start..end], 15.0);
            if x < (previous_width + width) * 0.5 {
                return start + offset;
            }
            previous_width = width;
        }
        draft.len()
    }
    fn color(c: u32) -> Color {
        Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
    }
    fn rect(&self, p: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: u32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = if w > 20.0 && h > 20.0 && w < p.width() as f32 / self.scale.get() - 50.0 {
            9.0_f32.min(h / 2.0)
        } else {
            0.0
        };
        let mut path = PathBuilder::new();
        path.move_to(x + r, y);
        path.line_to(x + w - r, y);
        path.quad_to(x + w, y, x + w, y + r);
        path.line_to(x + w, y + h - r);
        path.quad_to(x + w, y + h, x + w - r, y + h);
        path.line_to(x + r, y + h);
        path.quad_to(x, y + h, x, y + h - r);
        path.line_to(x, y + r);
        path.quad_to(x, y, x + r, y);
        path.close();
        if let Some(path) = path.finish() {
            let mut paint = Paint::default();
            paint.set_color(Self::color(c));
            p.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::from_scale(self.scale.get(), self.scale.get()),
                None,
            );
        }
    }
    fn caret_visible(&self) -> bool {
        self.elapsed.get() < self.force_caret_until.get()
            || ((self.elapsed.get() * 2.0) as u32).is_multiple_of(2)
    }
    fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (cx, cy) = self.cursor.get();
        cx >= x as f64 && cx < (x + w) as f64 && cy >= y as f64 && cy < (y + h) as f64
    }
    fn sound_chip(&self, p: &mut Pixmap) {
        let y = p.height() as f32 / self.scale.get() - 58.0;
        self.rect(
            p,
            14.0,
            y,
            40.0,
            40.0,
            if self.hovered(14.0, y, 40.0, 40.0) {
                Theme::AMBER
            } else {
                0x60463b
            },
        );
        self.text(
            p,
            if self.muted.get() { "OFF" } else { "SFX" },
            20.0,
            y + 25.0,
            11.0,
            Theme::CREAM,
            32.0,
        );
    }
    fn circle(&self, p: &mut Pixmap, x: f32, y: f32, r: f32, c: u32) {
        let mut paint = Paint::default();
        paint.set_color(Self::color(c));
        if let Some(path) = PathBuilder::from_circle(x, y, r) {
            p.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::from_scale(self.scale.get(), self.scale.get()),
                None,
            );
        }
    }
    fn leaf(&self, p: &mut Pixmap, x: f32, y: f32, c: u32) {
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
                Transform::from_scale(self.scale.get(), self.scale.get()),
                None,
            );
        }
    }
    #[allow(clippy::too_many_arguments)] // Raster primitive: position, style, and clipping width.
    fn text(&self, p: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, c: u32, max: f32) {
        let scale = self.scale.get();
        let size = size * scale;
        let x = x * scale;
        let y = y * scale;
        let max = max * scale;
        let mut pen = x;
        let mut previous = None;
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() > 2048 {
            cache.clear();
        }
        for ch in s.chars() {
            pen += previous
                .and_then(|p| self.font.horizontal_kern(p, ch, size))
                .unwrap_or(0.0);
            let (m, bitmap) = cache
                .entry((ch, size.to_bits()))
                .or_insert_with(|| self.font.rasterize_subpixel(ch, size));
            if pen + m.advance_width > x + max {
                break;
            }
            let origin_x = pen + m.xmin as f32;
            let origin_y = y - m.height as f32 - m.ymin as f32;
            // Preserve fractional placement. Average oversampled coverage to avoid RGB/BGR
            // assumptions and color fringes on rotated screens or screen captures.
            for py in (origin_y.floor() as i32)..((origin_y + m.height as f32).ceil() as i32) {
                if py < 0 || py >= p.height() as i32 {
                    continue;
                }
                for px in (origin_x.floor() as i32)..((origin_x + m.width as f32).ceil() as i32) {
                    if px < 0 || px >= p.width() as i32 || px as f32 >= x + max {
                        continue;
                    }
                    let mut coverage = 0.0;
                    for sample in 0..3 {
                        coverage += coverage_at(
                            bitmap,
                            m.width * 3,
                            m.height,
                            (px as f32 - origin_x) * 3.0 + sample as f32,
                            py as f32 - origin_y,
                        );
                    }
                    let alpha = (coverage / 3.0).round() as u32;
                    if alpha == 0 {
                        continue;
                    }
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
            previous = Some(ch);
        }
    }
    fn wrap(&self, s: &str, size: f32, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut width = 0.0;
        let mut previous = None;
        for c in s.chars() {
            let advance = self.font.metrics(c, size * self.scale.get()).advance_width
                / self.scale.get()
                + previous
                    .and_then(|p| self.font.horizontal_kern(p, c, size * self.scale.get()))
                    .unwrap_or(0.0)
                    / self.scale.get();
            if c == '\n' || width + advance > max {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
                previous = None;
            }
            if c != '\n' {
                line.push(c);
                width += if previous.is_none() {
                    self.font.metrics(c, size * self.scale.get()).advance_width / self.scale.get()
                } else {
                    advance
                };
                previous = Some(c);
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
        focused: bool,
    ) -> Vec<u8> {
        let mut p = Pixmap::new(
            self.dimensions(w, h).width(),
            self.dimensions(w, h).height(),
        )
        .expect("window pixmap");
        p.fill(Self::color(Theme::CREAM));
        let wf = w as f32;
        let hf = h as f32;
        self.rect(&mut p, 0.0, 0.0, 68.0, hf, Theme::BARK);
        self.circle(&mut p, 34.0, 45.0, 23.0, Theme::AMBER);
        self.leaf(&mut p, 34.0, 44.0, Theme::CREAM);
        self.circle(&mut p, 34.0, 110.0, 20.0, Theme::CRIMSON);
        self.circle(
            &mut p,
            34.0,
            154.0,
            20.0,
            if league == 1 { Theme::AMBER } else { 0x60463b },
        );
        self.text(&mut p, "T", 28.0, 161.0, 18.0, Theme::CREAM, 25.0);
        self.text(&mut p, "A", 28.0, 116.0, 20.0, Theme::CREAM, 30.0);
        self.rect(&mut p, 68.0, 0.0, 192.0, hf, 0xf3e6d5);
        self.rect(&mut p, 260.0, 0.0, 1.0, hf, Theme::GOLD);
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
            if *name == room || self.hovered(78.0, y, 174.0, 38.0) {
                self.rect(
                    &mut p,
                    78.0,
                    y,
                    174.0,
                    38.0,
                    if *name == room { 0xecc9a6 } else { 0xefddc7 },
                );
                if *name == room {
                    self.rect(&mut p, 78.0, y + 10.0, 3.0, 18.0, Theme::CRIMSON);
                }
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
        self.rect(
            &mut p,
            78.0,
            382.0,
            174.0,
            44.0,
            if voice || self.hovered(78.0, 382.0, 174.0, 44.0) {
                Theme::CRIMSON
            } else {
                Theme::AMBER
            },
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
            "COZY BY DESIGN",
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
                self.rect(&mut p, 78.0, y, 174.0, 32.0, 0xecc9a6);
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
            "Enter sends · F3 sounds",
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
        self.rect(&mut p, wf - 135.0, 14.0, 115.0, 45.0, Theme::CRIMSON);
        self.text(
            &mut p,
            "Sign out",
            wf - 121.0,
            43.0,
            14.0,
            Theme::CREAM,
            90.0,
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
        self.rect(&mut p, 260.0, 88.0, wf - 260.0, 1.0, Theme::GOLD);
        let member_x = wf - 155.0;
        let content_width = (member_x - 310.0).max(210.0);
        self.rect(&mut p, member_x, 89.0, 1.0, hf - 190.0, Theme::GOLD);
        self.text(
            &mut p,
            "YOUR STATUS",
            member_x + 15.0,
            121.0,
            11.0,
            0x806352,
            130.0,
        );
        self.circle(
            &mut p,
            member_x + 34.0,
            158.0,
            if speaking {
                22.0 + (self.elapsed.get() * 5.0).sin() * 2.0
            } else {
                18.0
            },
            if speaking { Theme::AMBER } else { Theme::GOLD },
        );
        self.circle(&mut p, member_x + 34.0, 158.0, 15.0, Theme::CRIMSON);
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
            self.circle(&mut p, member_x + 24.0, y - 4.0, 8.0, Theme::GOLD);
            self.text(&mut p, member, member_x + 42.0, y, 13.0, Theme::BARK, 100.0);
        }
        let mut lines = Vec::new();
        for (channel, author, text) in messages {
            if channel == room {
                lines.push((format!("{author}  •"), true));
                for line in self.wrap(text, 15.0, content_width - 45.0) {
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
                "Your local backend starts automatically. Say hello.",
                288.0,
                181.0,
                14.0,
                0x806352,
                content_width,
            );
        }
        for (i, (line, author)) in lines[start..end].iter().enumerate() {
            let y = 121.0 + i as f32 * 23.0;
            if *author {
                self.circle(&mut p, 296.0, y - 5.0, 11.0, Theme::CRIMSON);
                self.text(
                    &mut p,
                    &line.chars().next().unwrap_or('M').to_string(),
                    292.0,
                    y - 1.0,
                    11.0,
                    Theme::CREAM,
                    14.0,
                );
            } else if !line.is_empty() {
                self.rect(
                    &mut p,
                    313.0,
                    y - 18.0,
                    content_width - 28.0,
                    24.0,
                    0xf4e9da,
                );
            }
            self.text(
                &mut p,
                line,
                322.0,
                121.0 + i as f32 * 23.0,
                if *author { 13.0 } else { 15.0 },
                if *author { Theme::CRIMSON } else { Theme::BARK },
                content_width,
            );
        }
        self.rect(
            &mut p,
            280.0,
            hf - 91.0,
            wf - 300.0,
            48.0,
            if focused { Theme::AMBER } else { Theme::GOLD },
        );
        self.rect(&mut p, 281.0, hf - 90.0, wf - 302.0, 46.0, 0xffffff);
        self.rect(
            &mut p,
            wf - 106.0,
            hf - 90.0,
            85.0,
            46.0,
            if self.hovered(wf - 106.0, hf - 90.0, 85.0, 46.0) {
                Theme::CRIMSON
            } else {
                Theme::AMBER
            },
        );
        self.text(
            &mut p,
            "Send",
            wf - 88.0,
            hf - 61.0,
            15.0,
            Theme::CREAM,
            60.0,
        );
        let available = wf - 425.0;
        let caret = self.caret.get().min(draft.len());
        let mut start = self.view_start.get().min(caret);
        while !draft.is_char_boundary(start) {
            start -= 1;
        }
        while self.measure(&draft[start..caret], 15.0) > available - 12.0 {
            start += draft[start..].graphemes(true).next().map_or(0, str::len);
        }
        // Recover the left margin as the caret moves back toward the beginning.
        while start > 0 {
            let before = draft[..start]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i);
            if self.measure(&draft[before..caret], 15.0) > available - 12.0 {
                break;
            }
            start = before;
        }
        self.view_start.set(start);
        let mut end = caret;
        for (offset, ch) in draft[caret..].char_indices() {
            let candidate = caret + offset + ch.len_utf8();
            if self.measure(&draft[start..candidate], 15.0) > available - 4.0 {
                break;
            }
            end = candidate;
        }
        if let Some((a, b)) = self.selection.get().filter(|_| focused) {
            let a = a.max(start).min(end);
            let b = b.max(start).min(end);
            if a < b {
                let x = 296.0 + self.measure(&draft[start..a], 15.0);
                let width =
                    self.measure(&draft[start..b], 15.0) - self.measure(&draft[start..a], 15.0);
                self.rect(&mut p, x, hf - 80.0, width, 26.0, 0xf2d1ac);
            }
        }
        if draft.is_empty() {
            self.text(
                &mut p,
                "Write a message…",
                296.0,
                hf - 61.0,
                15.0,
                0x927767,
                available,
            );
        } else {
            self.text(
                &mut p,
                &draft[start..end],
                296.0,
                hf - 61.0,
                15.0,
                Theme::BARK,
                available,
            );
        }
        if focused && self.caret_visible() {
            self.rect(
                &mut p,
                296.0 + self.measure(&draft[start..caret], 15.0),
                hf - 79.0,
                1.2,
                23.0,
                Theme::AMBER,
            );
        }
        self.text(&mut p, status, 282.0, hf - 17.0, 11.0, 0x806352, wf - 300.0);
        self.sound_chip(&mut p);
        p.take()
    }
    pub fn about(&self, data: Vec<u8>, w: u32, h: u32, scroll: usize) -> Vec<u8> {
        let mut p = Pixmap::from_vec(data, self.dimensions(w, h)).unwrap();
        let wf = w as f32;
        let hf = h as f32;
        self.rect(&mut p, 78.0, 16.0, wf - 94.0, hf - 32.0, Theme::GOLD);
        self.rect(&mut p, 79.0, 17.0, wf - 96.0, hf - 34.0, Theme::CREAM);
        self.text(
            &mut p,
            "About Maple · Licenses",
            100.0,
            55.0,
            23.0,
            Theme::CRIMSON,
            wf - 140.0,
        );
        self.text(
            &mut p,
            "F1 or Escape closes · Scroll to read licenses",
            100.0,
            83.0,
            13.0,
            Theme::BARK,
            wf - 140.0,
        );
        let text=format!("Maple {}\nNative Rust UI, embedded server, statically linked Opus.\nArgon2id accounts and certificate-verified TLS 1.3 QUIC text/voice.\nF2 signs out. Encryption protects traffic to the server (not E2EE).\n\n{}\n\nApplication source license:\n{}",env!("CARGO_PKG_VERSION"),include_str!("../../../assets/fonts/OFL.txt"),include_str!("../../../LICENSE"));
        let lines = self.wrap(&text, 13.0, wf - 145.0);
        let capacity = ((hf - 135.0) / 20.0).max(1.0) as usize;
        let start = scroll.min(lines.len().saturating_sub(capacity));
        for (i, line) in lines.iter().skip(start).take(capacity).enumerate() {
            self.text(
                &mut p,
                line,
                100.0,
                119.0 + i as f32 * 20.0,
                13.0,
                Theme::BARK,
                wf - 145.0,
            );
        }
        p.take()
    }
}

fn coverage_at(bitmap: &[u8], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - ix as f32;
    let fy = y - iy as f32;
    let at = |x: i32, y: i32| {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            0.0
        } else {
            bitmap[y as usize * width + x as usize] as f32
        }
    };
    (at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx) * (1.0 - fy)
        + (at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx) * fy
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_text_placement_changes_coverage_without_color_fringes() {
        let painter = Painter::new().unwrap();
        let render = |x| {
            let mut p = Pixmap::new(150, 50).unwrap();
            p.fill(Color::WHITE);
            painter.text(&mut p, "AV office", x, 30.25, 16.0, 0, 130.0);
            p.take()
        };
        let integer = render(10.0);
        let fractional = render(10.35);
        assert_ne!(integer, fractional);
        assert!(fractional
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == p[1] && p[1] == p[2]));
        let ink = |data: &[u8]| {
            data.as_chunks::<4>()
                .0
                .iter()
                .map(|p| 255 - p[0] as u64)
                .sum::<u64>()
        };
        assert!(ink(&integer).abs_diff(ink(&fractional)) < 500);
    }
    #[test]
    fn fonts_rasterize_at_device_size_instead_of_scaling_finished_pixels() {
        let painter = Painter::new().unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            painter.scale.set(scale);
            painter
                .pixel_size
                .set(Some(((760.0 * scale) as u32, (520.0 * scale) as u32)));
            let data = painter.draw(
                760,
                520,
                "general",
                &VecDeque::new(),
                "AV office",
                "Ready",
                0,
                false,
                false,
                &[],
                &[],
                0,
                true,
            );
            let pixels = painter.dimensions(760, 520);
            assert_eq!(
                data.len(),
                pixels.width() as usize * pixels.height() as usize * 4
            );
            assert!(painter
                .glyphs
                .borrow()
                .contains_key(&('A', (15.0 * scale).to_bits())));
            let login = painter.sign_in(data, 760, 520, &crate::auth::Form::default(), "Ready");
            assert_eq!(
                login.len(),
                pixels.width() as usize * pixels.height() as usize * 4
            );
        }
    }
}
