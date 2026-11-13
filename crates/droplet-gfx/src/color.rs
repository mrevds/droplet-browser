//! RGBA color representation and CSS named/hex color parsing.

/// An 8-bit-per-channel RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Fully transparent black (`rgba(0, 0, 0, 0)`).
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    /// Construct a color with explicit red, green, blue, and alpha channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Construct an opaque color (`a = 255`) from red, green, and blue channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Pack the RGB channels into `0x00RRGGBB` format for XRGB framebuffers.
    pub const fn to_u32_xrgb(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Parse a CSS color name or `#`-prefixed hex literal (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`).
    pub fn from_name_or_hex(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if let Some(hex) = trimmed.strip_prefix('#') {
            return parse_hex_color(hex);
        }
        named_color(&trimmed.to_ascii_lowercase())
    }
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    if !hex.is_ascii() {
        return None;
    }
    let bytes = hex.as_bytes();
    match bytes.len() {
        3 => {
            let r = hex_nibble(bytes[0])?;
            let g = hex_nibble(bytes[1])?;
            let b = hex_nibble(bytes[2])?;
            Some(Color::rgb(r * 17, g * 17, b * 17))
        }
        4 => {
            let r = hex_nibble(bytes[0])?;
            let g = hex_nibble(bytes[1])?;
            let b = hex_nibble(bytes[2])?;
            let a = hex_nibble(bytes[3])?;
            Some(Color::rgba(r * 17, g * 17, b * 17, a * 17))
        }
        6 => {
            let r = hex_byte(bytes[0], bytes[1])?;
            let g = hex_byte(bytes[2], bytes[3])?;
            let b = hex_byte(bytes[4], bytes[5])?;
            Some(Color::rgb(r, g, b))
        }
        8 => {
            let r = hex_byte(bytes[0], bytes[1])?;
            let g = hex_byte(bytes[2], bytes[3])?;
            let b = hex_byte(bytes[4], bytes[5])?;
            let a = hex_byte(bytes[6], bytes[7])?;
            Some(Color::rgba(r, g, b, a))
        }
        _ => None,
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_nibble(hi)? << 4) | hex_nibble(lo)?)
}

fn named_color(name: &str) -> Option<Color> {
    let c = match name {
        "transparent" => Color::rgba(0, 0, 0, 0),
        "aliceblue" => Color::rgb(240, 248, 255),
        "antiquewhite" => Color::rgb(250, 235, 215),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "aquamarine" => Color::rgb(127, 255, 212),
        "azure" => Color::rgb(240, 255, 255),
        "beige" => Color::rgb(245, 245, 220),
        "bisque" => Color::rgb(255, 228, 196),
        "black" => Color::rgb(0, 0, 0),
        "blanchedalmond" => Color::rgb(255, 235, 205),
        "blue" => Color::rgb(0, 0, 255),
        "blueviolet" => Color::rgb(138, 43, 226),
        "brown" => Color::rgb(165, 42, 42),
        "burlywood" => Color::rgb(222, 184, 135),
        "cadetblue" => Color::rgb(95, 158, 160),
        "chartreuse" => Color::rgb(127, 255, 0),
        "chocolate" => Color::rgb(210, 105, 30),
        "coral" => Color::rgb(255, 127, 80),
        "cornflowerblue" => Color::rgb(100, 149, 237),
        "cornsilk" => Color::rgb(255, 248, 220),
        "crimson" => Color::rgb(220, 20, 60),
        "darkblue" => Color::rgb(0, 0, 139),
        "darkcyan" => Color::rgb(0, 139, 139),
        "darkgoldenrod" => Color::rgb(184, 134, 11),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "darkgreen" => Color::rgb(0, 100, 0),
        "darkkhaki" => Color::rgb(189, 183, 107),
        "darkmagenta" => Color::rgb(139, 0, 139),
        "darkolivegreen" => Color::rgb(85, 107, 47),
        "darkorange" => Color::rgb(255, 140, 0),
        "darkorchid" => Color::rgb(153, 50, 204),
        "darkred" => Color::rgb(139, 0, 0),
        "darksalmon" => Color::rgb(233, 150, 122),
        "darkseagreen" => Color::rgb(143, 188, 143),
        "darkslateblue" => Color::rgb(72, 61, 139),
        "darkslategray" | "darkslategrey" => Color::rgb(47, 79, 79),
        "darkturquoise" => Color::rgb(0, 206, 209),
        "darkviolet" => Color::rgb(148, 0, 211),
        "deeppink" => Color::rgb(255, 20, 147),
        "deepskyblue" => Color::rgb(0, 191, 255),
        "dimgray" | "dimgrey" => Color::rgb(105, 105, 105),
        "dodgerblue" => Color::rgb(30, 144, 255),
        "firebrick" => Color::rgb(178, 34, 34),
        "floralwhite" => Color::rgb(255, 250, 240),
        "forestgreen" => Color::rgb(34, 139, 34),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "gainsboro" => Color::rgb(220, 220, 220),
        "ghostwhite" => Color::rgb(248, 248, 255),
        "gold" => Color::rgb(255, 215, 0),
        "goldenrod" => Color::rgb(218, 165, 32),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "green" => Color::rgb(0, 128, 0),
        "greenyellow" => Color::rgb(173, 255, 47),
        "honeydew" => Color::rgb(240, 255, 240),
        "hotpink" => Color::rgb(255, 105, 180),
        "indianred" => Color::rgb(205, 92, 92),
        "indigo" => Color::rgb(75, 0, 130),
        "ivory" => Color::rgb(255, 255, 240),
        "khaki" => Color::rgb(240, 230, 140),
        "lavender" => Color::rgb(230, 230, 250),
        "lavenderblush" => Color::rgb(255, 240, 245),
        "lawngreen" => Color::rgb(124, 252, 0),
        "lemonchiffon" => Color::rgb(255, 250, 205),
        "lightblue" => Color::rgb(173, 216, 230),
        "lightcoral" => Color::rgb(240, 128, 128),
        "lightcyan" => Color::rgb(224, 255, 255),
        "lightgoldenrodyellow" => Color::rgb(250, 250, 210),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "lightgreen" => Color::rgb(144, 238, 144),
        "lightpink" => Color::rgb(255, 182, 193),
        "lightsalmon" => Color::rgb(255, 160, 122),
        "lightseagreen" => Color::rgb(32, 178, 170),
        "lightskyblue" => Color::rgb(135, 206, 250),
        "lightslategray" | "lightslategrey" => Color::rgb(119, 136, 153),
        "lightsteelblue" => Color::rgb(176, 196, 222),
        "lightyellow" => Color::rgb(255, 255, 224),
        "lime" => Color::rgb(0, 255, 0),
        "limegreen" => Color::rgb(50, 205, 50),
        "linen" => Color::rgb(250, 240, 230),
        "maroon" => Color::rgb(128, 0, 0),
        "mediumaquamarine" => Color::rgb(102, 205, 170),
        "mediumblue" => Color::rgb(0, 0, 205),
        "mediumorchid" => Color::rgb(186, 85, 211),
        "mediumpurple" => Color::rgb(147, 112, 219),
        "mediumseagreen" => Color::rgb(60, 179, 113),
        "mediumslateblue" => Color::rgb(123, 104, 238),
        "mediumspringgreen" => Color::rgb(0, 250, 154),
        "mediumturquoise" => Color::rgb(72, 209, 204),
        "mediumvioletred" => Color::rgb(199, 21, 133),
        "midnightblue" => Color::rgb(25, 25, 112),
        "mintcream" => Color::rgb(245, 255, 250),
        "mistyrose" => Color::rgb(255, 228, 225),
        "moccasin" => Color::rgb(255, 228, 181),
        "navajowhite" => Color::rgb(255, 222, 173),
        "navy" => Color::rgb(0, 0, 128),
        "oldlace" => Color::rgb(253, 245, 230),
        "olive" => Color::rgb(128, 128, 0),
        "olivedrab" => Color::rgb(107, 142, 35),
        "orange" => Color::rgb(255, 165, 0),
        "orangered" => Color::rgb(255, 69, 0),
        "orchid" => Color::rgb(218, 112, 214),
        "palegoldenrod" => Color::rgb(238, 232, 170),
        "palegreen" => Color::rgb(152, 251, 152),
        "paleturquoise" => Color::rgb(175, 238, 238),
        "palevioletred" => Color::rgb(219, 112, 147),
        "papayawhip" => Color::rgb(255, 239, 213),
        "peachpuff" => Color::rgb(255, 218, 185),
        "peru" => Color::rgb(205, 133, 63),
        "pink" => Color::rgb(255, 192, 203),
        "plum" => Color::rgb(221, 160, 221),
        "powderblue" => Color::rgb(176, 224, 230),
        "purple" => Color::rgb(128, 0, 128),
        "rebeccapurple" => Color::rgb(102, 51, 153),
        "red" => Color::rgb(255, 0, 0),
        "rosybrown" => Color::rgb(188, 143, 143),
        "royalblue" => Color::rgb(65, 105, 225),
        "saddlebrown" => Color::rgb(139, 69, 19),
        "salmon" => Color::rgb(250, 128, 114),
        "sandybrown" => Color::rgb(244, 164, 96),
        "seagreen" => Color::rgb(46, 139, 87),
        "seashell" => Color::rgb(255, 245, 238),
        "sienna" => Color::rgb(160, 82, 45),
        "silver" => Color::rgb(192, 192, 192),
        "skyblue" => Color::rgb(135, 206, 235),
        "slate" => Color::rgb(100, 116, 139),
        "slateblue" => Color::rgb(106, 90, 205),
        "slategray" | "slategrey" => Color::rgb(112, 128, 144),
        "snow" => Color::rgb(255, 250, 250),
        "springgreen" => Color::rgb(0, 255, 127),
        "steelblue" => Color::rgb(70, 130, 180),
        "tan" => Color::rgb(210, 180, 140),
        "teal" => Color::rgb(0, 128, 128),
        "thistle" => Color::rgb(216, 191, 216),
        "tomato" => Color::rgb(255, 99, 71),
        "turquoise" => Color::rgb(64, 224, 208),
        "violet" => Color::rgb(238, 130, 238),
        "wheat" => Color::rgb(245, 222, 179),
        "white" => Color::rgb(255, 255, 255),
        "whitesmoke" => Color::rgb(245, 245, 245),
        "yellow" => Color::rgb(255, 255, 0),
        "yellowgreen" => Color::rgb(154, 205, 50),
        _ => return None,
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn hex_and_named_colors_parse_correctly() {
        assert_eq!(Color::TRANSPARENT, Color::rgba(0, 0, 0, 0));
        assert_eq!(Color::default(), Color::TRANSPARENT);

        assert_eq!(Color::from_name_or_hex("#abc"), Some(Color::rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(Color::from_name_or_hex("#f08a"), Some(Color::rgba(0xff, 0x00, 0x88, 0xaa)));
        assert_eq!(Color::from_name_or_hex("#0f172a"), Some(Color::rgb(15, 23, 42)));
        assert_eq!(Color::from_name_or_hex("#ABCDEF"), Some(Color::rgb(0xab, 0xcd, 0xef)));
        assert_eq!(
            Color::from_name_or_hex("#ff000080"),
            Some(Color::rgba(255, 0, 0, 128))
        );
        assert_eq!(Color::from_name_or_hex("  #abc  "), Some(Color::rgb(0xaa, 0xbb, 0xcc)));

        assert_eq!(Color::from_name_or_hex("White"), Some(Color::rgb(255, 255, 255)));
        assert_eq!(Color::from_name_or_hex("  RebeccaPurple "), Some(Color::rgb(102, 51, 153)));
        assert_eq!(Color::from_name_or_hex("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(Color::from_name_or_hex("grey"), Color::from_name_or_hex("gray"));
        assert_eq!(Color::from_name_or_hex("slate"), Some(Color::rgb(100, 116, 139)));
        assert_eq!(Color::rgb(0x12, 0x34, 0x56).to_u32_xrgb(), 0x0012_3456);
    }

    #[test]
    fn invalid_colors_return_none() {
        for bad in ["", "#", "#1", "#12", "#12345", "#1234567", "#123456789", "#ggg", "#абв", "nope"] {
            assert_eq!(Color::from_name_or_hex(bad), None, "expected None for {bad:?}");
        }
    }
}
