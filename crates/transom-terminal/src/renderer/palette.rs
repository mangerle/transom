use super::*;

/// 只覆盖不随运行期变化的部分：命名色、终端 16 色、256 色立方与灰度回退。
/// 程序经 OSC 4/10/11 动态改写的调色板与 24 位 Spec 色仍是即时换算，
/// 换来的确定性失效复杂度远超其命中占比。
pub(crate) struct PaletteColors {
    pub(super) foreground: Hsla,
    pub(super) background: Hsla,
    pub(super) cursor: Hsla,
    pub(super) selection: Hsla,
    pub(super) ansi: [Hsla; 16],
    pub(super) ansi_dim: [Hsla; 16],
    pub(super) dim_foreground: Hsla,
    /// 256 色中 16..=255 的标准回退值（立方 + 灰度），下标为 index - 16。
    pub(super) indexed_fallback: [Hsla; 240],
}

impl PaletteColors {
    pub(crate) fn build(palette: &TerminalPalette) -> Self {
        let mut ansi = [Hsla::default(); 16];
        let mut ansi_dim = [Hsla::default(); 16];
        for slot in 0..16 {
            let base = palette_rgb(palette.ansi[slot]);
            ansi[slot] = base;
            let mut dimmed = base;
            dimmed.a *= 0.7;
            ansi_dim[slot] = dimmed;
        }
        let mut dim_foreground = palette_rgb(palette.foreground);
        dim_foreground.a *= 0.7;
        let mut indexed_fallback = [Hsla::default(); 240];
        for (slot, target) in indexed_fallback.iter_mut().enumerate() {
            let index = slot + 16;
            *target = if index < 232 {
                let cube = index - 16;
                let r = (cube / 36) as u8;
                let g = ((cube % 36) / 6) as u8;
                let b = (cube % 6) as u8;
                let channel = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
                palette_rgb(PaletteColor::new(channel(r), channel(g), channel(b)))
            } else {
                let level = 8 + (index - 232) as u8 * 10;
                palette_rgb(PaletteColor::new(level, level, level))
            };
        }
        Self {
            foreground: palette_rgb(palette.foreground),
            background: palette_rgb(palette.background),
            cursor: palette_rgb(palette.cursor),
            selection: palette_rgb(palette.selection),
            ansi,
            ansi_dim,
            dim_foreground,
            indexed_fallback,
        }
    }
}

/// RGB 分量直转 Hsla：与 `Hsla::from(rgb(to_hex()))` 数值完全一致，
/// 但省去 u32 打包/解包往返，供预计算与动态 Spec 色共用。
pub(crate) fn palette_rgb(color: PaletteColor) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: 1.0,
    })
}

impl TerminalRenderer {
    pub(crate) fn color(&self, color: PaletteColor) -> Hsla {
        palette_rgb(color)
    }

    pub(crate) fn resolve(&self, color: Color, colors: &Colors) -> Hsla {
        match color {
            Color::Spec(rgb) => self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b)),
            Color::Indexed(index) => self.resolve_indexed(index as usize, colors),
            Color::Named(named) => self.resolve_named(named, colors),
        }
    }

    fn resolve_named(&self, named: NamedColor, colors: &Colors) -> Hsla {
        if let Some(rgb) = colors[named] {
            return self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b));
        }
        let cached = &self.palette_colors;
        match named {
            NamedColor::Black
            | NamedColor::Red
            | NamedColor::Green
            | NamedColor::Yellow
            | NamedColor::Blue
            | NamedColor::Magenta
            | NamedColor::Cyan
            | NamedColor::White
            | NamedColor::BrightBlack
            | NamedColor::BrightRed
            | NamedColor::BrightGreen
            | NamedColor::BrightYellow
            | NamedColor::BrightBlue
            | NamedColor::BrightMagenta
            | NamedColor::BrightCyan
            | NamedColor::BrightWhite => cached.ansi[named as usize],
            NamedColor::Foreground | NamedColor::BrightForeground => cached.foreground,
            NamedColor::Background => cached.background,
            NamedColor::Cursor => cached.cursor,
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                cached.ansi_dim[named as usize - NamedColor::DimBlack as usize]
            }
            NamedColor::DimForeground => cached.dim_foreground,
        }
    }

    fn resolve_indexed(&self, index: usize, colors: &Colors) -> Hsla {
        if let Some(rgb) = colors[index] {
            return self.color(PaletteColor::new(rgb.r, rgb.g, rgb.b));
        }
        if index < 16 {
            return self.palette_colors.ansi[index];
        }
        // 256 色的立方与灰度回退不依赖运行期状态，直接查预计算表。
        self.palette_colors
            .indexed_fallback
            .get(index - 16)
            .copied()
            .unwrap_or_else(|| self.palette_colors.indexed_fallback[240 - 1])
    }

    pub(crate) fn cell_colors(&self, cell: &Cell, colors: &Colors) -> (Hsla, Hsla) {
        let mut fg = self.resolve(cell.fg, colors);
        let mut bg = self.resolve(cell.bg, colors);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg.a *= 0.7;
        }
        (fg, bg)
    }
}
