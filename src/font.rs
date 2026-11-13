/// Built-in 8x16 monochrome bitmap font covering printable ASCII (0x20..=0x7E),
/// full Russian Cyrillic (А-Я, а-я, Ё, ё), and common typographic symbols.
pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;

/// Measure the pixel width of a string rendered at the given integer scale.
pub fn measure_text(text: &str, scale: usize) -> f32 {
    text.chars().count() as f32 * (GLYPH_WIDTH * scale) as f32
}

/// Return the 8x16 bitmap (16 rows, MSB = leftmost pixel) for `ch`.
pub fn glyph_bitmap(ch: char) -> &'static [u8; 16] {
    match ch {
        '\u{0020}' | '\u{00A0}' => &GLYPH_SPACE,
        '!' => &GLYPH_EXCL,
        '"' | '“' | '”' => &GLYPH_QUOTE,
        '#' => &GLYPH_HASH,
        '$' => &GLYPH_DOLLAR,
        '%' => &GLYPH_PERCENT,
        '&' => &GLYPH_AMP,
        '\'' | '‘' | '’' => &GLYPH_APOS,
        '(' => &GLYPH_LPAREN,
        ')' => &GLYPH_RPAREN,
        '*' => &GLYPH_ASTERISK,
        '+' => &GLYPH_PLUS,
        ',' => &GLYPH_COMMA,
        '-' => &GLYPH_MINUS,
        '.' => &GLYPH_PERIOD,
        '/' => &GLYPH_SLASH,
        '0' => &GLYPH_0,
        '1' => &GLYPH_1,
        '2' => &GLYPH_2,
        '3' => &GLYPH_3,
        '4' => &GLYPH_4,
        '5' => &GLYPH_5,
        '6' => &GLYPH_6,
        '7' => &GLYPH_7,
        '8' => &GLYPH_8,
        '9' => &GLYPH_9,
        ':' => &GLYPH_COLON,
        ';' => &GLYPH_SEMICOLON,
        '<' => &GLYPH_LT,
        '=' => &GLYPH_EQ,
        '>' => &GLYPH_GT,
        '?' => &GLYPH_QUESTION,
        '@' => &GLYPH_AT,
        'A' => &GLYPH_A_UPPER,
        'B' => &GLYPH_B_UPPER,
        'C' => &GLYPH_C_UPPER,
        'D' => &GLYPH_D_UPPER,
        'E' => &GLYPH_E_UPPER,
        'F' => &GLYPH_F_UPPER,
        'G' => &GLYPH_G_UPPER,
        'H' => &GLYPH_H_UPPER,
        'I' => &GLYPH_I_UPPER,
        'J' => &GLYPH_J_UPPER,
        'K' => &GLYPH_K_UPPER,
        'L' => &GLYPH_L_UPPER,
        'M' => &GLYPH_M_UPPER,
        'N' => &GLYPH_N_UPPER,
        'O' => &GLYPH_O_UPPER,
        'P' => &GLYPH_P_UPPER,
        'Q' => &GLYPH_Q_UPPER,
        'R' => &GLYPH_R_UPPER,
        'S' => &GLYPH_S_UPPER,
        'T' => &GLYPH_T_UPPER,
        'U' => &GLYPH_U_UPPER,
        'V' => &GLYPH_V_UPPER,
        'W' => &GLYPH_W_UPPER,
        'X' => &GLYPH_X_UPPER,
        'Y' => &GLYPH_Y_UPPER,
        'Z' => &GLYPH_Z_UPPER,
        '[' => &GLYPH_LBRACKET,
        '\\' => &GLYPH_BACKSLASH,
        ']' => &GLYPH_RBRACKET,
        '^' => &GLYPH_CARET,
        '_' => &GLYPH_UNDERSCORE,
        '`' => &GLYPH_BACKTICK,
        'a' => &GLYPH_A_LOWER,
        'b' => &GLYPH_B_LOWER,
        'c' => &GLYPH_C_LOWER,
        'd' => &GLYPH_D_LOWER,
        'e' => &GLYPH_E_LOWER,
        'f' => &GLYPH_F_LOWER,
        'g' => &GLYPH_G_LOWER,
        'h' => &GLYPH_H_LOWER,
        'i' => &GLYPH_I_LOWER,
        'j' => &GLYPH_J_LOWER,
        'k' => &GLYPH_K_LOWER,
        'l' => &GLYPH_L_LOWER,
        'm' => &GLYPH_M_LOWER,
        'n' => &GLYPH_N_LOWER,
        'o' => &GLYPH_O_LOWER,
        'p' => &GLYPH_P_LOWER,
        'q' => &GLYPH_Q_LOWER,
        'r' => &GLYPH_R_LOWER,
        's' => &GLYPH_S_LOWER,
        't' => &GLYPH_T_LOWER,
        'u' => &GLYPH_U_LOWER,
        'v' => &GLYPH_V_LOWER,
        'w' => &GLYPH_W_LOWER,
        'x' => &GLYPH_X_LOWER,
        'y' => &GLYPH_Y_LOWER,
        'z' => &GLYPH_Z_LOWER,
        '{' => &GLYPH_LBRACE,
        '|' => &GLYPH_PIPE,
        '}' => &GLYPH_RBRACE,
        '~' => &GLYPH_TILDE,

        // Russian Cyrillic uppercase
        'А' => &GLYPH_A_UPPER,
        'Б' => &GLYPH_CYR_BE_UPPER,
        'В' => &GLYPH_B_UPPER,
        'Г' => &GLYPH_CYR_GE_UPPER,
        'Д' => &GLYPH_CYR_DE_UPPER,
        'Е' => &GLYPH_E_UPPER,
        'Ё' => &GLYPH_CYR_YO_UPPER,
        'Ж' => &GLYPH_CYR_ZHE_UPPER,
        'З' => &GLYPH_CYR_ZE_UPPER,
        'И' => &GLYPH_CYR_I_UPPER,
        'Й' => &GLYPH_CYR_SHORT_I_UPPER,
        'К' => &GLYPH_K_UPPER,
        'Л' => &GLYPH_CYR_EL_UPPER,
        'М' => &GLYPH_M_UPPER,
        'Н' => &GLYPH_H_UPPER,
        'О' => &GLYPH_O_UPPER,
        'П' => &GLYPH_CYR_PE_UPPER,
        'Р' => &GLYPH_P_UPPER,
        'С' => &GLYPH_C_UPPER,
        'Т' => &GLYPH_T_UPPER,
        'У' => &GLYPH_CYR_U_UPPER,
        'Ф' => &GLYPH_CYR_EF_UPPER,
        'Х' => &GLYPH_X_UPPER,
        'Ц' => &GLYPH_CYR_TSE_UPPER,
        'Ч' => &GLYPH_CYR_CHE_UPPER,
        'Ш' => &GLYPH_CYR_SHA_UPPER,
        'Щ' => &GLYPH_CYR_SHCHA_UPPER,
        'Ъ' => &GLYPH_CYR_HARD_UPPER,
        'Ы' => &GLYPH_CYR_YERU_UPPER,
        'Ь' => &GLYPH_CYR_SOFT_UPPER,
        'Э' => &GLYPH_CYR_E_UPPER,
        'Ю' => &GLYPH_CYR_YU_UPPER,
        'Я' => &GLYPH_CYR_YA_UPPER,

        // Russian Cyrillic lowercase
        'а' => &GLYPH_A_LOWER,
        'б' => &GLYPH_CYR_BE_LOWER,
        'в' => &GLYPH_CYR_VE_LOWER,
        'г' => &GLYPH_CYR_GE_LOWER,
        'д' => &GLYPH_CYR_DE_LOWER,
        'е' => &GLYPH_E_LOWER,
        'ё' => &GLYPH_CYR_YO_LOWER,
        'ж' => &GLYPH_CYR_ZHE_LOWER,
        'з' => &GLYPH_CYR_ZE_LOWER,
        'и' => &GLYPH_CYR_I_LOWER,
        'й' => &GLYPH_CYR_SHORT_I_LOWER,
        'к' => &GLYPH_CYR_KA_LOWER,
        'л' => &GLYPH_CYR_EL_LOWER,
        'м' => &GLYPH_M_LOWER,
        'н' => &GLYPH_CYR_EN_LOWER,
        'о' => &GLYPH_O_LOWER,
        'п' => &GLYPH_CYR_PE_LOWER,
        'р' => &GLYPH_P_LOWER,
        'с' => &GLYPH_C_LOWER,
        'т' => &GLYPH_CYR_TE_LOWER,
        'у' => &GLYPH_Y_LOWER,
        'ф' => &GLYPH_CYR_EF_LOWER,
        'х' => &GLYPH_X_LOWER,
        'ц' => &GLYPH_CYR_TSE_LOWER,
        'ч' => &GLYPH_CYR_CHE_LOWER,
        'ш' => &GLYPH_CYR_SHA_LOWER,
        'щ' => &GLYPH_CYR_SHCHA_LOWER,
        'ъ' => &GLYPH_CYR_HARD_LOWER,
        'ы' => &GLYPH_CYR_YERU_LOWER,
        'ь' => &GLYPH_CYR_SOFT_LOWER,
        'э' => &GLYPH_CYR_E_LOWER,
        'ю' => &GLYPH_CYR_YU_LOWER,
        'я' => &GLYPH_CYR_YA_LOWER,

        // Typographic punctuation and arrows
        '«' => &GLYPH_LAQUO,
        '»' => &GLYPH_RAQUO,
        '—' => &GLYPH_EM_DASH,
        '–' => &GLYPH_EN_DASH,
        '→' => &GLYPH_ARROW_RIGHT,
        '←' => &GLYPH_ARROW_LEFT,
        '•' => &GLYPH_BULLET,
        '…' => &GLYPH_ELLIPSIS,
        '°' => &GLYPH_DEGREE,
        '№' => &GLYPH_NUMERO,
        '©' => &GLYPH_COPYRIGHT,

        _ => &GLYPH_FALLBACK,
    }
}

// ============================================================================
// ASCII 0x20..=0x7E
// Baseline = row 12, cap height = rows 3..=12, x-height = rows 6..=12
// ============================================================================

const GLYPH_SPACE: [u8; 16] = [0x00; 16];

const GLYPH_FALLBACK: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x42, 0x42, 0x42, 0x42,
    0x42, 0x42, 0x42, 0x42, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_EXCL: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18,
    0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_QUOTE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x24, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_HASH: [u8; 16] = [
    0x00, 0x00, 0x00, 0x24, 0x24, 0x7E, 0x24, 0x24,
    0x24, 0x7E, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_DOLLAR: [u8; 16] = [
    0x00, 0x00, 0x18, 0x3C, 0x5A, 0x50, 0x38, 0x1C,
    0x1A, 0x5A, 0x3C, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_PERCENT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x62, 0x64, 0x08, 0x08, 0x10,
    0x10, 0x20, 0x4C, 0x8C, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_AMP: [u8; 16] = [
    0x00, 0x00, 0x00, 0x38, 0x4C, 0x4C, 0x38, 0x70,
    0x6A, 0x64, 0x6A, 0x3E, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_APOS: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x18, 0x18, 0x30, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_LPAREN: [u8; 16] = [
    0x00, 0x00, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x18, 0x0C, 0x00, 0x00, 0x00,
];

const GLYPH_RPAREN: [u8; 16] = [
    0x00, 0x00, 0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00, 0x00, 0x00,
];

const GLYPH_ASTERISK: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x18, 0x5A, 0x3C, 0x7E,
    0x3C, 0x5A, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_PLUS: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x7E,
    0x18, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_COMMA: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x18, 0x18, 0x18, 0x30, 0x00, 0x00,
];

const GLYPH_MINUS: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E,
    0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_PERIOD: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_SLASH: [u8; 16] = [
    0x00, 0x00, 0x00, 0x06, 0x06, 0x0C, 0x0C, 0x18,
    0x18, 0x30, 0x30, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_0: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x6E, 0x76,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_1: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x38, 0x78, 0x18, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_2: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x06, 0x0C,
    0x18, 0x30, 0x60, 0x66, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_3: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x06, 0x06, 0x1C,
    0x06, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_4: [u8; 16] = [
    0x00, 0x00, 0x00, 0x0C, 0x1C, 0x3C, 0x6C, 0x6C,
    0x7E, 0x0C, 0x0C, 0x0C, 0x0C, 0x00, 0x00, 0x00,
];

const GLYPH_5: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x60, 0x60, 0x60, 0x7C,
    0x06, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_6: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x60, 0x60, 0x60, 0x7C,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_7: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x66, 0x06, 0x0C, 0x0C,
    0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_8: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_9: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3E,
    0x06, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_COLON: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18,
    0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_SEMICOLON: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18,
    0x00, 0x00, 0x00, 0x18, 0x18, 0x30, 0x00, 0x00,
];

const GLYPH_LT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x06, 0x0C, 0x18, 0x30, 0x60,
    0x30, 0x18, 0x0C, 0x06, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_EQ: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00,
    0x00, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_GT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x30, 0x18, 0x0C, 0x06,
    0x0C, 0x18, 0x30, 0x60, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_QUESTION: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x0C, 0x18,
    0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_AT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7C, 0x82, 0x9E, 0xA2, 0xA2,
    0xA2, 0x9C, 0x80, 0x82, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_A_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x3C, 0x66, 0x66, 0x66,
    0x7E, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_B_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x7C,
    0x66, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_C_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x60, 0x60, 0x60,
    0x60, 0x60, 0x60, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_D_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x78, 0x6C, 0x66, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x6C, 0x78, 0x00, 0x00, 0x00,
];

const GLYPH_E_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x60, 0x60, 0x60, 0x7C,
    0x60, 0x60, 0x60, 0x60, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_F_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x60, 0x60, 0x60, 0x7C,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_G_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x60, 0x60, 0x60,
    0x6E, 0x66, 0x66, 0x66, 0x3E, 0x00, 0x00, 0x00,
];

const GLYPH_H_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x7E,
    0x66, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_I_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x18, 0x18, 0x18, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_J_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x1E, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0C, 0x0C, 0x6C, 0x6C, 0x38, 0x00, 0x00, 0x00,
];

const GLYPH_K_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x6C, 0x78, 0x70,
    0x78, 0x6C, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_L_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x60, 0x60, 0x60,
    0x60, 0x60, 0x60, 0x60, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_M_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0xC6, 0xEE, 0xFE, 0xD6, 0xC6,
    0xC6, 0xC6, 0xC6, 0xC6, 0xC6, 0x00, 0x00, 0x00,
];

const GLYPH_N_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x76, 0x76, 0x7E,
    0x6E, 0x6E, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_O_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_P_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x7C,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_Q_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x66,
    0x66, 0x66, 0x6E, 0x6C, 0x3E, 0x06, 0x00, 0x00,
];

const GLYPH_R_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x7C,
    0x6C, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_S_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x60, 0x60, 0x3C,
    0x06, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_T_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x7E, 0x18, 0x18, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_U_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_V_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x66,
    0x66, 0x3C, 0x3C, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_W_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0xC6, 0xC6, 0xC6, 0xC6, 0xC6,
    0xD6, 0xD6, 0xFE, 0xEE, 0xC6, 0x00, 0x00, 0x00,
];

const GLYPH_X_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x3C, 0x3C, 0x18,
    0x3C, 0x3C, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_Y_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_Z_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x06, 0x0C, 0x0C, 0x18,
    0x30, 0x30, 0x60, 0x60, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_LBRACKET: [u8; 16] = [
    0x00, 0x00, 0x3C, 0x30, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_BACKSLASH: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x30, 0x30, 0x18,
    0x18, 0x0C, 0x0C, 0x06, 0x06, 0x00, 0x00, 0x00,
];

const GLYPH_RBRACKET: [u8; 16] = [
    0x00, 0x00, 0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CARET: [u8; 16] = [
    0x00, 0x00, 0x18, 0x3C, 0x66, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_UNDERSCORE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00, 0x00,
];

const GLYPH_BACKTICK: [u8; 16] = [
    0x00, 0x00, 0x30, 0x18, 0x0C, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_A_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x06,
    0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00, 0x00, 0x00,
];

const GLYPH_B_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x60, 0x7C, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_C_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66,
    0x60, 0x60, 0x60, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_D_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x06, 0x06, 0x06, 0x3E, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3E, 0x00, 0x00, 0x00,
];

const GLYPH_E_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66,
    0x7E, 0x60, 0x60, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_F_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x1C, 0x36, 0x30, 0x7C, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x00, 0x00, 0x00,
];

const GLYPH_G_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x66,
    0x66, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00,
];

const GLYPH_H_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x60, 0x7C, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_I_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x38, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_J_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00, 0x1C, 0x0C,
    0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00,
];

const GLYPH_K_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x60, 0x66, 0x6C,
    0x78, 0x78, 0x6C, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_L_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x38, 0x18, 0x18, 0x18, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_M_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEC, 0xFE,
    0xD6, 0xD6, 0xD6, 0xD6, 0xD6, 0x00, 0x00, 0x00,
];

const GLYPH_N_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_O_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_P_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x66,
    0x66, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00,
];

const GLYPH_Q_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x66,
    0x66, 0x66, 0x66, 0x3E, 0x06, 0x06, 0x06, 0x00,
];

const GLYPH_R_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5C, 0x76,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_S_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x60,
    0x3C, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_T_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x30, 0x30, 0x7C, 0x30,
    0x30, 0x30, 0x30, 0x36, 0x1C, 0x00, 0x00, 0x00,
];

const GLYPH_U_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3E, 0x00, 0x00, 0x00,
];

const GLYPH_V_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x66, 0x66, 0x3C, 0x3C, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_W_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC6, 0xC6,
    0xD6, 0xD6, 0xD6, 0xFE, 0x6C, 0x00, 0x00, 0x00,
];

const GLYPH_X_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_Y_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00,
];

const GLYPH_Z_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x0C,
    0x18, 0x18, 0x30, 0x30, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_LBRACE: [u8; 16] = [
    0x00, 0x00, 0x1C, 0x30, 0x30, 0x30, 0x30, 0x60,
    0x30, 0x30, 0x30, 0x30, 0x1C, 0x00, 0x00, 0x00,
];

const GLYPH_PIPE: [u8; 16] = [
    0x00, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18,
    0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00,
];

const GLYPH_RBRACE: [u8; 16] = [
    0x00, 0x00, 0x38, 0x0C, 0x0C, 0x0C, 0x0C, 0x06,
    0x0C, 0x0C, 0x0C, 0x0C, 0x38, 0x00, 0x00, 0x00,
];

const GLYPH_TILDE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x32, 0x7C,
    0x4C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

// ============================================================================
// Russian Cyrillic Uppercase
// ============================================================================

const GLYPH_CYR_BE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x60, 0x60, 0x60, 0x7C,
    0x66, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_GE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x62, 0x60, 0x60, 0x60,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_DE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3E, 0x36, 0x36, 0x36, 0x36,
    0x36, 0x66, 0x66, 0x7E, 0xFE, 0x82, 0x82, 0x00,
];

const GLYPH_CYR_YO_UPPER: [u8; 16] = [
    0x24, 0x24, 0x00, 0x7E, 0x60, 0x60, 0x60, 0x7C,
    0x60, 0x60, 0x60, 0x60, 0x7E, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_ZHE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x92, 0x92, 0x54, 0x54, 0x38,
    0x38, 0x54, 0x54, 0x92, 0x92, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_ZE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x06, 0x06, 0x1C,
    0x06, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_I_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x6E, 0x6E, 0x7E,
    0x76, 0x76, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHORT_I_UPPER: [u8; 16] = [
    0x22, 0x1C, 0x00, 0x66, 0x66, 0x6E, 0x6E, 0x7E,
    0x76, 0x76, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_EL_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3E, 0x3E, 0x36, 0x36, 0x36,
    0x36, 0x66, 0x66, 0x66, 0xC6, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_PE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x7E, 0x7E, 0x66, 0x66, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_U_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x66,
    0x3E, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_EF_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x10, 0x10, 0x7C, 0x92, 0x92,
    0x92, 0x92, 0x7C, 0x10, 0x10, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_TSE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x6C, 0x6C, 0x6C, 0x6C, 0x6C,
    0x6C, 0x6C, 0x6C, 0x7E, 0x7E, 0x06, 0x06, 0x00,
];

const GLYPH_CYR_CHE_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHA_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x92, 0x92, 0x92, 0x92, 0x92,
    0x92, 0x92, 0x92, 0xFE, 0xFE, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHCHA_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0xA8, 0xA8, 0xA8, 0xA8, 0xA8,
    0xA8, 0xA8, 0xA8, 0xFE, 0xFE, 0x06, 0x06, 0x00,
];

const GLYPH_CYR_HARD_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0xE0, 0xE0, 0x30, 0x30, 0x3C,
    0x36, 0x36, 0x36, 0x36, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YERU_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x86, 0x86, 0x86, 0x86, 0xE6,
    0x96, 0x96, 0x96, 0x96, 0xE6, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SOFT_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x60, 0x60, 0x60, 0x60, 0x7C,
    0x66, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_E_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x66, 0x06, 0x06, 0x3E,
    0x3E, 0x06, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YU_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x9C, 0xA2, 0xA2, 0xA2, 0xE2,
    0xE2, 0xA2, 0xA2, 0xA2, 0x9C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YA_UPPER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3E, 0x66, 0x66, 0x66, 0x3E,
    0x36, 0x36, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

// ============================================================================
// Russian Cyrillic Lowercase
// ============================================================================

const GLYPH_CYR_BE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x0E, 0x38, 0x60, 0x7C, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_VE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x66,
    0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_GE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x62,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_DE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x36,
    0x36, 0x36, 0x66, 0x7E, 0xFE, 0x82, 0x82, 0x00,
];

const GLYPH_CYR_YO_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x24, 0x24, 0x00, 0x3C, 0x66,
    0x7E, 0x60, 0x60, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_ZHE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x54,
    0x38, 0x38, 0x54, 0x92, 0x92, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_ZE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66,
    0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_I_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x6E,
    0x7E, 0x76, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHORT_I_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x22, 0x1C, 0x00, 0x66, 0x6E,
    0x7E, 0x76, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_KA_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x6C,
    0x78, 0x78, 0x6C, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_EL_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x36,
    0x36, 0x36, 0x66, 0x66, 0xC6, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_EN_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x66, 0x7E, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_PE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x66,
    0x66, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_TE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x7E,
    0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_EF_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x10, 0x7C, 0x92,
    0x92, 0x92, 0x92, 0x7C, 0x10, 0x10, 0x10, 0x00,
];

const GLYPH_CYR_TSE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6C, 0x6C,
    0x6C, 0x6C, 0x6C, 0x7E, 0x7E, 0x06, 0x06, 0x00,
];

const GLYPH_CYR_CHE_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x66,
    0x66, 0x3E, 0x06, 0x06, 0x06, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHA_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x92,
    0x92, 0x92, 0x92, 0xFE, 0xFE, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SHCHA_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA8, 0xA8,
    0xA8, 0xA8, 0xA8, 0xFE, 0xFE, 0x06, 0x06, 0x00,
];

const GLYPH_CYR_HARD_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE0, 0x30,
    0x3C, 0x36, 0x36, 0x36, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YERU_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x86, 0x86,
    0xE6, 0x96, 0x96, 0x96, 0xE6, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_SOFT_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x60, 0x60,
    0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_E_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66,
    0x06, 0x3E, 0x06, 0x66, 0x3C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YU_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9C, 0xA2,
    0xE2, 0xE2, 0xA2, 0xA2, 0x9C, 0x00, 0x00, 0x00,
];

const GLYPH_CYR_YA_LOWER: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x66,
    0x66, 0x3E, 0x36, 0x66, 0x66, 0x00, 0x00, 0x00,
];

// ============================================================================
// Typographic Symbols & Arrows
// ============================================================================

const GLYPH_LAQUO: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x24,
    0x48, 0x48, 0x24, 0x12, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_RAQUO: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x24,
    0x12, 0x12, 0x24, 0x48, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_EM_DASH: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF,
    0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_EN_DASH: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_ARROW_RIGHT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x0C, 0x7E,
    0x7E, 0x0C, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_ARROW_LEFT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x30, 0x7E,
    0x7E, 0x30, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_BULLET: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x3C,
    0x3C, 0x3C, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_ELLIPSIS: [u8; 16] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x92, 0x92, 0x00, 0x00, 0x00,
];

const GLYPH_DEGREE: [u8; 16] = [
    0x00, 0x00, 0x00, 0x18, 0x24, 0x24, 0x18, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const GLYPH_NUMERO: [u8; 16] = [
    0x00, 0x00, 0x00, 0x92, 0xD5, 0xD5, 0xB2, 0x90,
    0x97, 0x90, 0x90, 0x90, 0x90, 0x00, 0x00, 0x00,
];

const GLYPH_COPYRIGHT: [u8; 16] = [
    0x00, 0x00, 0x00, 0x3C, 0x42, 0x99, 0xA5, 0xA1,
    0xA5, 0x99, 0x42, 0x3C, 0x00, 0x00, 0x00, 0x00,
];

// ============================================================================
// System TTF Discovery, Pure-Rust sfnt/glyf Rasterizer & Bounded GlyphAtlas
// ============================================================================

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const MAX_ATLAS_ENTRIES: usize = 4096;
const MAX_COMPOUND_DEPTH: usize = 8;
const RASTER_SUB_ROWS: usize = 16;

/// Rasterized anti-aliased glyph bitmap with layout metrics.
#[derive(Debug, Clone)]
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    /// Offset in pixels from the top of the line box (`y + bearing_y`).
    pub bearing_y: i32,
    pub advance: f32,
    /// Alpha coverage `0..=255`, length `width * height`.
    pub alpha: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FontSlot {
    Regular = 0,
    Bold = 1,
    MonoRegular = 2,
    MonoBold = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    font_slot: u8,
    ch: char,
    px_tenths: u32,
    bold_synth: bool,
}

#[derive(Clone)]
struct FontSet {
    regular: Option<Arc<TrueTypeFont>>,
    bold: Option<(Arc<TrueTypeFont>, bool)>,
    mono_regular: Option<Arc<TrueTypeFont>>,
    mono_bold: Option<(Arc<TrueTypeFont>, bool)>,
    fallback: Vec<Arc<TrueTypeFont>>,
}

impl FontSet {
    fn empty() -> Self {
        Self {
            regular: None,
            bold: None,
            mono_regular: None,
            mono_bold: None,
            fallback: Vec::new(),
        }
    }

    fn discover() -> Self {
        let regular_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSans[wdth,wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];
        let bold_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
        ];
        let mono_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSansMono[wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            r"C:\Windows\Fonts\consola.ttf",
            r"C:\Windows\Fonts\cour.ttf",
        ];
        let mono_bold_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-B.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
            r"C:\Windows\Fonts\consolab.ttf",
            r"C:\Windows\Fonts\courbd.ttf",
        ];
        let fallback_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];

        let (regular, reg_canon) = load_first_ttf(regular_candidates);
        let bold = load_bold_variant(bold_candidates, &regular, reg_canon.as_ref());

        let (mono_regular, mono_canon) = load_first_ttf(mono_candidates);
        let mono_bold = load_bold_variant(mono_bold_candidates, &mono_regular, mono_canon.as_ref());

        let mut fallback = Vec::new();
        let mut seen_paths = Vec::new();
        if let Some(p) = reg_canon {
            seen_paths.push(p);
        }
        for path in fallback_candidates {
            if let Ok(canon) = std::fs::canonicalize(path) {
                if seen_paths.contains(&canon) {
                    continue;
                }
                if let Some(font) = load_ttf_file(path) {
                    seen_paths.push(canon);
                    fallback.push(font);
                }
            }
        }

        Self {
            regular,
            bold,
            mono_regular,
            mono_bold,
            fallback,
        }
    }

    fn has_any(&self) -> bool {
        self.regular.is_some()
            || self.bold.is_some()
            || self.mono_regular.is_some()
            || self.mono_bold.is_some()
    }

    fn slot_for(bold: bool, mono: bool) -> FontSlot {
        match (bold, mono) {
            (false, false) => FontSlot::Regular,
            (true, false) => FontSlot::Bold,
            (false, true) => FontSlot::MonoRegular,
            (true, true) => FontSlot::MonoBold,
        }
    }

    fn resolve_slot(&self, slot: FontSlot) -> (Option<&Arc<TrueTypeFont>>, bool) {
        match slot {
            FontSlot::Regular => (self.regular.as_ref(), false),
            FontSlot::Bold => match &self.bold {
                Some((f, synth)) => (Some(f), *synth),
                None => (self.regular.as_ref(), true),
            },
            FontSlot::MonoRegular => (self.mono_regular.as_ref().or(self.regular.as_ref()), false),
            FontSlot::MonoBold => match &self.mono_bold {
                Some((f, synth)) => (Some(f), *synth),
                None => (self.mono_regular.as_ref().or(self.regular.as_ref()), true),
            },
        }
    }

    fn resolve_for_char(&self, ch: char, slot: FontSlot) -> (Option<&Arc<TrueTypeFont>>, u16, bool) {
        let (primary, bold_synth) = self.resolve_slot(slot);
        let wants_bold = matches!(slot, FontSlot::Bold | FontSlot::MonoBold);
        let lookup_ch = if ch == '\u{00A0}' { ' ' } else { ch };

        if let Some(font) = primary {
            let gid = font.glyph_index(lookup_ch);
            if gid != 0 {
                return (Some(font), gid, bold_synth);
            }
        }
        for fb in &self.fallback {
            let gid = fb.glyph_index(lookup_ch);
            if gid != 0 {
                return (Some(fb), gid, wants_bold);
            }
        }
        (None, 0, wants_bold)
    }

    fn line_height(&self, font_size: f32) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        if let Some(font) = self.regular.as_ref().or(self.mono_regular.as_ref()) {
            font.line_height(fs)
        } else {
            fs * 1.25
        }
    }

    fn baseline_offset(&self, font_size: f32) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        if let Some(font) = self.regular.as_ref().or(self.mono_regular.as_ref()) {
            font.baseline_from_top_f32(fs)
        } else {
            fs * 0.85
        }
    }

    fn measure_text(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 || text.is_empty() {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        let slot = Self::slot_for(bold, mono);
        let mut total = 0.0f32;
        for ch in text.chars() {
            total += self.char_advance(ch, fs, slot);
        }
        total
    }

    fn char_advance(&self, ch: char, font_size: f32, slot: FontSlot) -> f32 {
        if matches!(ch, '\n' | '\r') {
            return 0.0;
        }
        let (font_opt, gid, bold_synth) = self.resolve_for_char(ch, slot);
        if let Some(font) = font_opt {
            let base = font.advance_px(gid, font_size);
            if bold_synth {
                base + synthetic_bold_extra_px(font_size)
            } else {
                base
            }
        } else {
            let scale = (font_size / GLYPH_HEIGHT as f32).max(0.25);
            let mut adv = GLYPH_WIDTH as f32 * scale;
            if bold_synth && ch != ' ' && ch != '\u{00A0}' {
                adv += 1.0;
            }
            adv
        }
    }
}

fn load_ttf_file(path: &str) -> Option<Arc<TrueTypeFont>> {
    let bytes = std::fs::read(path).ok()?;
    TrueTypeFont::parse(bytes).ok().map(Arc::new)
}

fn load_first_ttf(candidates: &[&str]) -> (Option<Arc<TrueTypeFont>>, Option<PathBuf>) {
    for path in candidates {
        if let Some(font) = load_ttf_file(path) {
            let canon = std::fs::canonicalize(path).ok();
            return (Some(font), canon);
        }
    }
    (None, None)
}

fn load_bold_variant(
    bold_candidates: &[&str],
    regular: &Option<Arc<TrueTypeFont>>,
    regular_canon: Option<&PathBuf>,
) -> Option<(Arc<TrueTypeFont>, bool)> {
    let first_path = bold_candidates.first().copied();
    if let (Some(reg), Some(reg_path), Some(p)) = (regular, regular_canon, first_path) {
        if let Ok(bold_canon) = std::fs::canonicalize(p) {
            if &bold_canon == reg_path {
                // Variable-font symlink (e.g. Ubuntu-B.ttf -> Ubuntu[wdth,wght].ttf):
                // keep the matching typeface and synthesize bold weight cleanly.
                return Some((Arc::clone(reg), true));
            }
        }
    }
    for path in bold_candidates {
        if let Ok(canon) = std::fs::canonicalize(path) {
            if regular_canon == Some(&canon) {
                if let Some(reg) = regular {
                    return Some((Arc::clone(reg), true));
                }
            }
        }
        if let Some(font) = load_ttf_file(path) {
            return Some((font, false));
        }
    }
    regular.as_ref().map(|reg| (Arc::clone(reg), true))
}

static SYSTEM_FONTS: OnceLock<Arc<FontSet>> = OnceLock::new();

fn system_font_set() -> Arc<FontSet> {
    Arc::clone(SYSTEM_FONTS.get_or_init(|| Arc::new(FontSet::discover())))
}

/// Fast read-only proportional text measurement using the discovered system fonts (or 8x16 fallback).
pub fn measure_text_px(text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
    system_font_set().measure_text(text, font_size, bold, mono)
}

/// Fast read-only line height calculation using discovered system fonts (or 8x16 fallback).
pub fn line_height_px(font_size: f32) -> f32 {
    system_font_set().line_height(font_size)
}

/// Offset from the top of the line box to the typographic baseline in pixels.
pub fn baseline_offset_px(font_size: f32) -> f32 {
    system_font_set().baseline_offset(font_size)
}

/// Font rasterizer engine with system TTF discovery, pure-Rust glyph rasterization,
/// 8x16 bitmap fallback, and a bounded glyph atlas cache.
pub struct FontEngine {
    fonts: Arc<FontSet>,
    atlas: HashMap<GlyphCacheKey, RasterGlyph>,
}

impl Default for FontEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FontEngine {
    /// Create a `FontEngine` backed by discovered system TrueType fonts (with 8x16 fallback).
    pub fn new() -> Self {
        Self {
            fonts: system_font_set(),
            atlas: HashMap::new(),
        }
    }

    /// Creates a fallback-only FontEngine (for testing fallback deterministically).
    pub fn fallback_only() -> Self {
        Self {
            fonts: Arc::new(FontSet::empty()),
            atlas: HashMap::new(),
        }
    }

    pub fn has_system_font(&self) -> bool {
        self.fonts.has_any()
    }

    pub fn line_height(&self, font_size: f32) -> f32 {
        self.fonts.line_height(font_size)
    }

    pub fn measure_text(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        self.fonts.measure_text(text, font_size, bold, mono)
    }

    pub fn rasterize_glyph(
        &mut self,
        ch: char,
        font_size: f32,
        bold: bool,
        mono: bool,
    ) -> &RasterGlyph {
        let fs = if font_size.is_finite() && font_size > 0.0 {
            font_size.clamp(1.0, 512.0)
        } else {
            0.0
        };
        let px_tenths = (fs * 10.0).round() as u32;
        let slot = FontSet::slot_for(bold, mono);
        let (_, _, bold_synth) = self.fonts.resolve_for_char(ch, slot);
        let key = GlyphCacheKey {
            font_slot: slot as u8,
            ch,
            px_tenths,
            bold_synth,
        };

        if !self.atlas.contains_key(&key) {
            if self.atlas.len() >= MAX_ATLAS_ENTRIES {
                self.atlas.clear();
            }
            let glyph = if fs <= 0.0 {
                RasterGlyph {
                    width: 0,
                    height: 0,
                    bearing_x: 0,
                    bearing_y: 0,
                    advance: 0.0,
                    alpha: Vec::new(),
                }
            } else {
                self.render_uncached(ch, fs, slot)
            };
            self.atlas.insert(key, glyph);
        }

        self.atlas
            .get(&key)
            .expect("cached glyph must exist immediately after insertion")
    }

    fn render_uncached(&self, ch: char, font_size: f32, slot: FontSlot) -> RasterGlyph {
        if matches!(ch, '\n' | '\r') {
            return RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance: 0.0,
                alpha: Vec::new(),
            };
        }
        let (font_opt, gid, bold_synth) = self.fonts.resolve_for_char(ch, slot);
        if let Some(font) = font_opt {
            if let Some(glyph) = font.rasterize_glyph(gid, font_size, bold_synth) {
                return glyph;
            }
        }
        rasterize_fallback_glyph(ch, font_size, bold_synth)
    }
}

fn synthetic_bold_extra_px(font_size: f32) -> f32 {
    (font_size * 0.055).max(0.75)
}

fn rasterize_fallback_glyph(ch: char, font_size: f32, bold: bool) -> RasterGlyph {
    let scale = (font_size / GLYPH_HEIGHT as f32).max(0.25);
    let base_w = ((GLYPH_WIDTH as f32 * scale).round() as u32).max(1);
    let h = ((GLYPH_HEIGHT as f32 * scale).round() as u32).max(1);

    if ch == ' ' || ch == '\u{00A0}' {
        return RasterGlyph {
            width: 0,
            height: 0,
            bearing_x: 0,
            bearing_y: 0,
            advance: GLYPH_WIDTH as f32 * scale,
            alpha: Vec::new(),
        };
    }

    let extra_w = if bold { 1 } else { 0 };
    let w = base_w + extra_w;
    let advance = GLYPH_WIDTH as f32 * scale + extra_w as f32;
    let bits = glyph_bitmap(ch);
    let mut alpha = vec![0u8; (w * h) as usize];

    for py in 0..h {
        let row_idx = ((py * GLYPH_HEIGHT as u32) / h).min(15) as usize;
        let row_bits = bits[row_idx];
        if row_bits == 0 {
            continue;
        }
        for px in 0..w {
            let mut lit = false;
            if px < base_w {
                let col = ((px * GLYPH_WIDTH as u32) / base_w).min(7) as usize;
                lit = (row_bits & (0x80 >> col)) != 0;
            }
            if !lit && bold && px > 0 && (px - 1) < base_w {
                let prev_col = (((px - 1) * GLYPH_WIDTH as u32) / base_w).min(7) as usize;
                lit = (row_bits & (0x80 >> prev_col)) != 0;
            }
            if lit {
                alpha[(py * w + px) as usize] = 255;
            }
        }
    }

    RasterGlyph {
        width: w,
        height: h,
        bearing_x: 0,
        bearing_y: 0,
        advance,
        alpha,
    }
}

// ============================================================================
// Pure-Rust TrueType (sfnt / glyf) Parser & Anti-Aliased Scanline Rasterizer
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct TableRange {
    offset: usize,
    length: usize,
}

#[derive(Debug, Clone)]
struct TrueTypeFont {
    data: Vec<u8>,
    units_per_em: u16,
    index_to_loc_format: i16,
    num_glyphs: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    num_h_metrics: u16,
    cmap4_range: Option<TableRange>,
    cmap12_range: Option<TableRange>,
    hmtx_range: TableRange,
    loca_range: TableRange,
    glyf_range: TableRange,
}

impl TrueTypeFont {
    fn parse(data: Vec<u8>) -> Result<Self, &'static str> {
        if data.len() < 12 {
            return Err("TTF header truncated");
        }
        let sfnt_ver = read_u32_be(&data, 0)?;
        if sfnt_ver != 0x0001_0000 && sfnt_ver != 0x7472_7565 {
            return Err("unsupported sfnt version");
        }
        let num_tables = read_u16_be(&data, 4)? as usize;
        if num_tables == 0 || num_tables > 256 {
            return Err("invalid sfnt table count");
        }
        let dir_end = 12usize
            .checked_add(num_tables * 16)
            .ok_or("sfnt table directory overflow")?;
        if dir_end > data.len() {
            return Err("truncated sfnt table directory");
        }

        let mut head_r = None;
        let mut maxp_r = None;
        let mut cmap_r = None;
        let mut hhea_r = None;
        let mut hmtx_r = None;
        let mut loca_r = None;
        let mut glyf_r = None;

        for i in 0..num_tables {
            let rec = 12 + i * 16;
            let tag = &data[rec..rec + 4];
            let offset = read_u32_be(&data, rec + 8)? as usize;
            let length = read_u32_be(&data, rec + 12)? as usize;
            let end = offset.checked_add(length).ok_or("table offset overflow")?;
            if end > data.len() {
                return Err("table extends past end of TTF");
            }
            let range = TableRange { offset, length };
            match tag {
                b"head" => head_r = Some(range),
                b"maxp" => maxp_r = Some(range),
                b"cmap" => cmap_r = Some(range),
                b"hhea" => hhea_r = Some(range),
                b"hmtx" => hmtx_r = Some(range),
                b"loca" => loca_r = Some(range),
                b"glyf" => glyf_r = Some(range),
                _ => {}
            }
        }

        let head_r = head_r.ok_or("missing head table")?;
        let maxp_r = maxp_r.ok_or("missing maxp table")?;
        let cmap_r = cmap_r.ok_or("missing cmap table")?;
        let hhea_r = hhea_r.ok_or("missing hhea table")?;
        let hmtx_range = hmtx_r.ok_or("missing hmtx table")?;
        let loca_range = loca_r.ok_or("missing loca table")?;
        let glyf_range = glyf_r.ok_or("missing glyf table")?;

        if head_r.length < 54 {
            return Err("truncated head table");
        }
        let head = &data[head_r.offset..head_r.offset + head_r.length];
        let units_per_em = read_u16_be(head, 18)?;
        if !(16..=16384).contains(&units_per_em) {
            return Err("invalid unitsPerEm");
        }
        let index_to_loc_format = read_i16_be(head, 50)?;
        if index_to_loc_format != 0 && index_to_loc_format != 1 {
            return Err("invalid indexToLocFormat");
        }

        if maxp_r.length < 6 {
            return Err("truncated maxp table");
        }
        let maxp = &data[maxp_r.offset..maxp_r.offset + maxp_r.length];
        let num_glyphs = read_u16_be(maxp, 4)?;
        if num_glyphs == 0 {
            return Err("zero glyphs in maxp");
        }

        if hhea_r.length < 36 {
            return Err("truncated hhea table");
        }
        let hhea = &data[hhea_r.offset..hhea_r.offset + hhea_r.length];
        let ascent = read_i16_be(hhea, 4)?;
        let descent = read_i16_be(hhea, 6)?;
        let line_gap = read_i16_be(hhea, 8)?;
        let num_h_metrics = read_u16_be(hhea, 34)?;
        if num_h_metrics == 0 || (num_h_metrics as usize) * 4 > hmtx_range.length {
            return Err("invalid hmtx metrics length");
        }

        let needed_loca = if index_to_loc_format == 0 {
            (num_glyphs as usize + 1) * 2
        } else {
            (num_glyphs as usize + 1) * 4
        };
        if loca_range.length < needed_loca {
            return Err("truncated loca table");
        }

        let (cmap4_range, cmap12_range) = parse_cmap_ranges(&data, cmap_r)?;
        if cmap4_range.is_none() && cmap12_range.is_none() {
            return Err("no supported cmap format 4 or 12 subtable");
        }

        Ok(Self {
            data,
            units_per_em,
            index_to_loc_format,
            num_glyphs,
            ascent,
            descent,
            line_gap,
            num_h_metrics,
            cmap4_range,
            cmap12_range,
            hmtx_range,
            loca_range,
            glyf_range,
        })
    }

    fn line_height(&self, font_size: f32) -> f32 {
        let scale = font_size / self.units_per_em as f32;
        let raw = (self.ascent as i32 - self.descent as i32 + self.line_gap as i32).max(1) as f32
            * scale;
        raw.max(font_size * 1.2)
    }

    fn baseline_from_top_f32(&self, font_size: f32) -> f32 {
        let scale = font_size / self.units_per_em as f32;
        let ascent_px = self.ascent as f32 * scale;
        let descent_px = self.descent as f32 * scale;
        let lh = self.line_height(font_size);
        let extra = (lh - (ascent_px - descent_px)).max(0.0);
        ascent_px + extra * 0.5
    }

    fn baseline_from_top(&self, font_size: f32) -> i32 {
        self.baseline_from_top_f32(font_size).round() as i32
    }

    fn glyph_index(&self, ch: char) -> u16 {
        let cp = ch as u32;
        if let Some(r12) = self.cmap12_range {
            if let Some(gid) = lookup_cmap12(&self.data[r12.offset..r12.offset + r12.length], cp) {
                if gid < self.num_glyphs as u32 {
                    return gid as u16;
                }
            }
        }
        if cp <= 0xFFFF {
            if let Some(r4) = self.cmap4_range {
                if let Some(gid) =
                    lookup_cmap4(&self.data[r4.offset..r4.offset + r4.length], cp as u16)
                {
                    if gid < self.num_glyphs {
                        return gid;
                    }
                }
            }
        }
        0
    }

    fn advance_units(&self, gid: u16) -> u16 {
        let hmtx = &self.data[self.hmtx_range.offset..self.hmtx_range.offset + self.hmtx_range.length];
        let idx = (gid.min(self.num_h_metrics - 1)) as usize * 4;
        read_u16_be(hmtx, idx).unwrap_or(0)
    }

    fn advance_px(&self, gid: u16, font_size: f32) -> f32 {
        let units = self.advance_units(gid) as f32;
        units * (font_size / self.units_per_em as f32)
    }

    fn glyph_slice(&self, gid: u16) -> Option<&[u8]> {
        if gid >= self.num_glyphs {
            return None;
        }
        let loca = &self.data[self.loca_range.offset..self.loca_range.offset + self.loca_range.length];
        let (start, end) = if self.index_to_loc_format == 0 {
            let i = gid as usize * 2;
            let s = read_u16_be(loca, i).ok()? as usize * 2;
            let e = read_u16_be(loca, i + 2).ok()? as usize * 2;
            (s, e)
        } else {
            let i = gid as usize * 4;
            let s = read_u32_be(loca, i).ok()? as usize;
            let e = read_u32_be(loca, i + 4).ok()? as usize;
            (s, e)
        };
        if start >= end || end > self.glyf_range.length {
            return None;
        }
        let abs_start = self.glyf_range.offset + start;
        let abs_end = self.glyf_range.offset + end;
        Some(&self.data[abs_start..abs_end])
    }

    fn rasterize_glyph(
        &self,
        gid: u16,
        font_size: f32,
        bold_synth: bool,
    ) -> Option<RasterGlyph> {
        let scale = font_size / self.units_per_em as f32;
        let mut advance = self.advance_px(gid, font_size);
        let synth_px = if bold_synth {
            synthetic_bold_extra_px(font_size)
        } else {
            0.0
        };
        if bold_synth {
            advance += synth_px;
        }

        let mut segments = Vec::new();
        self.collect_glyph_segments(gid, Transform2D::IDENTITY, scale, 0, &mut segments);
        if segments.is_empty() {
            return Some(RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance,
                alpha: Vec::new(),
            });
        }

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for &((ax, ay), (bx, by)) in &segments {
            min_x = min_x.min(ax).min(bx);
            min_y = min_y.min(ay).min(by);
            max_x = max_x.max(ax).max(bx);
            max_y = max_y.max(ay).max(by);
        }
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            return None;
        }

        let x0 = min_x.floor() as i32;
        let y0 = min_y.floor() as i32;
        let x1 = max_x.ceil() as i32;
        let y1 = max_y.ceil() as i32;

        let base_w = (x1 - x0).clamp(0, 1024) as usize;
        let h = (y1 - y0).clamp(0, 1024) as usize;
        if base_w == 0 || h == 0 {
            return Some(RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance,
                alpha: Vec::new(),
            });
        }

        let mut alpha = scanline_rasterize(&segments, x0, y0, base_w, h);
        let mut final_w = base_w;
        if bold_synth {
            let (dilated, new_w) = apply_synthetic_bold(&alpha, base_w, h, synth_px);
            alpha = dilated;
            final_w = new_w;
        }

        let baseline_px = self.baseline_from_top(font_size);
        Some(RasterGlyph {
            width: final_w as u32,
            height: h as u32,
            bearing_x: x0,
            bearing_y: baseline_px + y0,
            advance,
            alpha,
        })
    }

    fn collect_glyph_segments(
        &self,
        gid: u16,
        xform: Transform2D,
        scale: f32,
        depth: usize,
        out: &mut Vec<Segment2D>,
    ) {
        if depth >= MAX_COMPOUND_DEPTH {
            return;
        }
        let Some(g) = self.glyph_slice(gid) else {
            return;
        };
        if g.len() < 10 {
            return;
        }
        let Ok(num_contours) = read_i16_be(g, 0) else {
            return;
        };

        if num_contours >= 0 {
            decode_simple_glyph(g, num_contours as usize, xform, scale, out);
        } else {
            self.decode_compound_glyph(g, xform, scale, depth, out);
        }
    }

    fn decode_compound_glyph(
        &self,
        g: &[u8],
        parent_xform: Transform2D,
        scale: f32,
        depth: usize,
        out: &mut Vec<Segment2D>,
    ) {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let mut pos = 10usize;
        loop {
            let Ok(flags) = read_u16_be(g, pos) else {
                return;
            };
            let Ok(sub_gid) = read_u16_be(g, pos + 2) else {
                return;
            };
            pos += 4;

            let (dx, dy) = if (flags & ARG_1_AND_2_ARE_WORDS) != 0 {
                let Ok(a1) = read_i16_be(g, pos) else {
                    return;
                };
                let Ok(a2) = read_i16_be(g, pos + 2) else {
                    return;
                };
                pos += 4;
                if (flags & ARGS_ARE_XY_VALUES) != 0 {
                    (a1 as f32, a2 as f32)
                } else {
                    (0.0, 0.0)
                }
            } else {
                if pos + 2 > g.len() {
                    return;
                }
                let a1 = g[pos] as i8;
                let a2 = g[pos + 1] as i8;
                pos += 2;
                if (flags & ARGS_ARE_XY_VALUES) != 0 {
                    (a1 as f32, a2 as f32)
                } else {
                    (0.0, 0.0)
                }
            };

            let mut comp = Transform2D {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: dx,
                ty: dy,
            };

            if (flags & WE_HAVE_A_SCALE) != 0 {
                let Ok(s) = read_f2dot14(g, pos) else {
                    return;
                };
                pos += 2;
                comp.a = s;
                comp.d = s;
            } else if (flags & WE_HAVE_AN_X_AND_Y_SCALE) != 0 {
                let Ok(sx) = read_f2dot14(g, pos) else {
                    return;
                };
                let Ok(sy) = read_f2dot14(g, pos + 2) else {
                    return;
                };
                pos += 4;
                comp.a = sx;
                comp.d = sy;
            } else if (flags & WE_HAVE_A_TWO_BY_TWO) != 0 {
                let Ok(a) = read_f2dot14(g, pos) else {
                    return;
                };
                let Ok(b) = read_f2dot14(g, pos + 2) else {
                    return;
                };
                let Ok(c) = read_f2dot14(g, pos + 4) else {
                    return;
                };
                let Ok(d) = read_f2dot14(g, pos + 6) else {
                    return;
                };
                pos += 8;
                comp.a = a;
                comp.b = b;
                comp.c = c;
                comp.d = d;
            }

            let combined = comp.then(&parent_xform);
            self.collect_glyph_segments(sub_gid, combined, scale, depth + 1, out);

            if (flags & MORE_COMPONENTS) == 0 {
                break;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Transform2D {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Transform2D {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    fn then(&self, parent: &Self) -> Self {
        Self {
            a: parent.a * self.a + parent.c * self.b,
            b: parent.b * self.a + parent.d * self.b,
            c: parent.a * self.c + parent.c * self.d,
            d: parent.b * self.c + parent.d * self.d,
            tx: parent.a * self.tx + parent.c * self.ty + parent.tx,
            ty: parent.b * self.tx + parent.d * self.ty + parent.ty,
        }
    }
}

type Segment2D = ((f32, f32), (f32, f32));

#[derive(Clone, Copy)]
struct ContourPoint {
    x: f32,
    y: f32,
    on_curve: bool,
}

fn decode_simple_glyph(
    g: &[u8],
    num_contours: usize,
    xform: Transform2D,
    scale: f32,
    out: &mut Vec<Segment2D>,
) {
    if num_contours == 0 {
        return;
    }
    let mut pos = 10usize;
    let Some(end_pts_end) = pos.checked_add(num_contours * 2) else {
        return;
    };
    if end_pts_end + 2 > g.len() {
        return;
    }

    let mut end_pts = Vec::with_capacity(num_contours);
    let mut prev_end = -1i32;
    for i in 0..num_contours {
        let Ok(ep) = read_u16_be(g, pos + i * 2) else {
            return;
        };
        if (ep as i32) < prev_end {
            return;
        }
        prev_end = ep as i32;
        end_pts.push(ep as usize);
    }
    pos = end_pts_end;

    let num_points = match end_pts.last() {
        Some(&last) if last < 65535 => last + 1,
        _ => return,
    };

    let Ok(instr_len) = read_u16_be(g, pos) else {
        return;
    };
    pos += 2;
    pos = match pos.checked_add(instr_len as usize) {
        Some(p) if p <= g.len() => p,
        _ => return,
    };

    let mut flags = Vec::with_capacity(num_points);
    while flags.len() < num_points {
        if pos >= g.len() {
            return;
        }
        let flag = g[pos];
        pos += 1;
        flags.push(flag);
        if (flag & 0x08) != 0 {
            if pos >= g.len() {
                return;
            }
            let repeat = g[pos] as usize;
            pos += 1;
            if flags.len() + repeat > num_points {
                return;
            }
            flags.resize(flags.len() + repeat, flag);
        }
    }

    let mut xs = vec![0i32; num_points];
    let mut cur_x = 0i32;
    for i in 0..num_points {
        let f = flags[i];
        let short_x = (f & 0x02) != 0;
        let same_or_pos = (f & 0x10) != 0;
        if short_x {
            if pos >= g.len() {
                return;
            }
            let d = g[pos] as i32;
            pos += 1;
            cur_x += if same_or_pos { d } else { -d };
        } else if !same_or_pos {
            let Ok(d) = read_i16_be(g, pos) else {
                return;
            };
            pos += 2;
            cur_x += d as i32;
        }
        xs[i] = cur_x;
    }

    let mut ys = vec![0i32; num_points];
    let mut cur_y = 0i32;
    for i in 0..num_points {
        let f = flags[i];
        let short_y = (f & 0x04) != 0;
        let same_or_pos = (f & 0x20) != 0;
        if short_y {
            if pos >= g.len() {
                return;
            }
            let d = g[pos] as i32;
            pos += 1;
            cur_y += if same_or_pos { d } else { -d };
        } else if !same_or_pos {
            let Ok(d) = read_i16_be(g, pos) else {
                return;
            };
            pos += 2;
            cur_y += d as i32;
        }
        ys[i] = cur_y;
    }

    let mut start = 0usize;
    for &end_idx in &end_pts {
        if end_idx >= num_points || end_idx < start {
            return;
        }
        let contour: Vec<ContourPoint> = (start..=end_idx)
            .map(|i| {
                let (fx, fy) = xform.apply(xs[i] as f32, ys[i] as f32);
                ContourPoint {
                    x: fx * scale,
                    y: -fy * scale,
                    on_curve: (flags[i] & 0x01) != 0,
                }
            })
            .collect();
        flatten_contour(&contour, out);
        start = end_idx + 1;
    }
}

fn flatten_contour(pts: &[ContourPoint], out: &mut Vec<Segment2D>) {
    let n = pts.len();
    if n < 2 {
        return;
    }

    let (mut cur, start_step, end_step) = if pts[0].on_curve {
        ((pts[0].x, pts[0].y), 1usize, n)
    } else if pts[n - 1].on_curve {
        ((pts[n - 1].x, pts[n - 1].y), 0usize, n - 1)
    } else {
        let mid = (
            (pts[n - 1].x + pts[0].x) * 0.5,
            (pts[n - 1].y + pts[0].y) * 0.5,
        );
        (mid, 0usize, n - 1)
    };

    let mut step = start_step;
    while step <= end_step {
        let p = pts[step % n];
        if p.on_curve {
            let next = (p.x, p.y);
            if cur != next {
                out.push((cur, next));
            }
            cur = next;
            step += 1;
        } else {
            let ctrl = (p.x, p.y);
            let next_pt = pts[(step + 1) % n];
            if next_pt.on_curve {
                let next = (next_pt.x, next_pt.y);
                flatten_quad_bezier(cur, ctrl, next, 0, out);
                cur = next;
                step += 2;
            } else {
                let mid = ((ctrl.0 + next_pt.x) * 0.5, (ctrl.1 + next_pt.y) * 0.5);
                flatten_quad_bezier(cur, ctrl, mid, 0, out);
                cur = mid;
                step += 1;
            }
        }
    }
}

fn flatten_quad_bezier(
    p0: (f32, f32),
    ctrl: (f32, f32),
    p1: (f32, f32),
    depth: u8,
    out: &mut Vec<Segment2D>,
) {
    let dev_x = (p0.0 + p1.0) * 0.5 - ctrl.0;
    let dev_y = (p0.1 + p1.1) * 0.5 - ctrl.1;
    let err_sq = dev_x * dev_x + dev_y * dev_y;
    if depth >= 8 || err_sq <= 0.125 * 0.125 {
        if p0 != p1 {
            out.push((p0, p1));
        }
        return;
    }
    let m0 = ((p0.0 + ctrl.0) * 0.5, (p0.1 + ctrl.1) * 0.5);
    let m1 = ((ctrl.0 + p1.0) * 0.5, (ctrl.1 + p1.1) * 0.5);
    let mid = ((m0.0 + m1.0) * 0.5, (m0.1 + m1.1) * 0.5);
    flatten_quad_bezier(p0, m0, mid, depth + 1, out);
    flatten_quad_bezier(mid, m1, p1, depth + 1, out);
}

fn scanline_rasterize(
    segments: &[Segment2D],
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut local_segs: Vec<(f32, f32, f32, f32, f32, f32)> = Vec::with_capacity(segments.len());
    for &((ax, ay), (bx, by)) in segments {
        let lax = ax - x0 as f32;
        let lay = ay - y0 as f32;
        let lbx = bx - x0 as f32;
        let lby = by - y0 as f32;
        if lay != lby {
            let min_y = lay.min(lby);
            let max_y = lay.max(lby);
            local_segs.push((lax, lay, lbx, lby, min_y, max_y));
        }
    }

    let mut alpha = vec![0u8; width * height];
    let mut row_cov = vec![0.0f32; width];
    let mut row_segs: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(32);
    let mut crossings: Vec<(f32, i32)> = Vec::with_capacity(32);
    let w_f32 = width as f32;

    for row in 0..height {
        let row_top = row as f32;
        let row_bot = (row + 1) as f32;
        row_segs.clear();
        for &(ax, ay, bx, by, min_y, max_y) in &local_segs {
            if max_y > row_top && min_y < row_bot {
                row_segs.push((ax, ay, bx, by));
            }
        }
        if row_segs.is_empty() {
            continue;
        }

        row_cov.fill(0.0);
        for sub in 0..RASTER_SUB_ROWS {
            let sy = row_top + (sub as f32 + 0.5) / RASTER_SUB_ROWS as f32;
            crossings.clear();
            for &(ax, ay, bx, by) in &row_segs {
                if ay <= sy && sy < by {
                    let x = ax + (sy - ay) * (bx - ax) / (by - ay);
                    crossings.push((x, 1));
                } else if by <= sy && sy < ay {
                    let x = bx + (sy - by) * (ax - bx) / (ay - by);
                    crossings.push((x, -1));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding = 0i32;
            let mut span_start = 0.0f32;
            for &(x, dir) in &crossings {
                let prev_w = winding;
                winding += dir;
                if prev_w == 0 && winding != 0 {
                    span_start = x;
                } else if prev_w != 0 && winding == 0 {
                    let s = span_start.clamp(0.0, w_f32);
                    let e = x.clamp(0.0, w_f32);
                    if e > s {
                        let p_start = s.floor() as usize;
                        let p_end = (e.ceil() as usize).min(width);
                        if p_start < p_end {
                            if p_start + 1 == p_end {
                                row_cov[p_start] += e - s;
                            } else {
                                row_cov[p_start] += (p_start + 1) as f32 - s;
                                for cell in &mut row_cov[(p_start + 1)..(p_end - 1)] {
                                    *cell += 1.0;
                                }
                                row_cov[p_end - 1] += e - (p_end - 1) as f32;
                            }
                        }
                    }
                }
            }
        }

        let dst_row = &mut alpha[row * width..(row + 1) * width];
        for (px, &cov) in row_cov.iter().enumerate() {
            let v = ((cov / RASTER_SUB_ROWS as f32) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            dst_row[px] = v;
        }
    }

    alpha
}

fn apply_synthetic_bold(alpha: &[u8], width: usize, height: usize, synth_px: f32) -> (Vec<u8>, usize) {
    let extra = (synth_px.ceil() as usize).max(1);
    let new_w = width + extra;
    let mut out = vec![0u8; new_w * height];
    let whole = synth_px.floor() as usize;
    let frac = synth_px - whole as f32;

    for y in 0..height {
        let src_row = &alpha[y * width..(y + 1) * width];
        let dst_row = &mut out[y * new_w..(y + 1) * new_w];
        for x in 0..new_w {
            let mut val = if x < width { src_row[x] } else { 0 };
            for d in 1..=whole {
                if x >= d && (x - d) < width {
                    val = val.max(src_row[x - d]);
                }
            }
            if frac > 0.0 && x > whole && (x - whole - 1) < width {
                let shifted = (src_row[x - whole - 1] as f32 * frac).round() as u8;
                val = val.max(shifted);
            }
            dst_row[x] = val;
        }
    }
    (out, new_w)
}

fn parse_cmap_ranges(
    data: &[u8],
    cmap_r: TableRange,
) -> Result<(Option<TableRange>, Option<TableRange>), &'static str> {
    if cmap_r.length < 4 {
        return Err("truncated cmap table");
    }
    let cmap = &data[cmap_r.offset..cmap_r.offset + cmap_r.length];
    let num_tables = read_u16_be(cmap, 2)? as usize;
    if 4 + num_tables * 8 > cmap.len() {
        return Err("truncated cmap encoding records");
    }

    let mut best4: Option<(u8, TableRange)> = None;
    let mut best12: Option<(u8, TableRange)> = None;

    for i in 0..num_tables {
        let rec = 4 + i * 8;
        let platform = read_u16_be(cmap, rec)?;
        let encoding = read_u16_be(cmap, rec + 2)?;
        let sub_rel = read_u32_be(cmap, rec + 4)? as usize;
        if sub_rel + 4 > cmap.len() {
            continue;
        }
        let abs_off = cmap_r.offset + sub_rel;
        let sub_avail = cmap.len() - sub_rel;
        let fmt = read_u16_be(cmap, sub_rel)?;

        match fmt {
            4 => {
                let sub_len = read_u16_be(cmap, sub_rel + 2)? as usize;
                if sub_len < 16 || sub_len > sub_avail {
                    continue;
                }
                let prio = match (platform, encoding) {
                    (3, 1) => 3,
                    (0, 3) => 2,
                    (0, _) => 1,
                    _ => 0,
                };
                if best4.is_none_or(|(p, _)| prio > p) {
                    best4 = Some((
                        prio,
                        TableRange {
                            offset: abs_off,
                            length: sub_len,
                        },
                    ));
                }
            }
            12 => {
                if sub_avail < 16 {
                    continue;
                }
                let sub_len = read_u32_be(cmap, sub_rel + 4)? as usize;
                if sub_len < 16 || sub_len > sub_avail {
                    continue;
                }
                let prio = match (platform, encoding) {
                    (3, 10) => 3,
                    (0, 4) => 2,
                    (0, _) => 1,
                    _ => 0,
                };
                if best12.is_none_or(|(p, _)| prio > p) {
                    best12 = Some((
                        prio,
                        TableRange {
                            offset: abs_off,
                            length: sub_len,
                        },
                    ));
                }
            }
            _ => {}
        }
    }

    Ok((best4.map(|(_, r)| r), best12.map(|(_, r)| r)))
}

fn lookup_cmap4(sub: &[u8], cp: u16) -> Option<u16> {
    if sub.len() < 16 {
        return None;
    }
    let seg_count_x2 = read_u16_be(sub, 6).ok()? as usize;
    if seg_count_x2 < 2 || !seg_count_x2.is_multiple_of(2) {
        return None;
    }
    let seg_count = seg_count_x2 / 2;
    let end_codes_off = 14usize;
    let start_codes_off = end_codes_off + seg_count * 2 + 2;
    let id_delta_off = start_codes_off + seg_count * 2;
    let id_range_off = id_delta_off + seg_count * 2;
    if id_range_off + seg_count * 2 > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = seg_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let end_code = read_u16_be(sub, end_codes_off + mid * 2).ok()?;
        if end_code < cp {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= seg_count {
        return Some(0);
    }

    let start_code = read_u16_be(sub, start_codes_off + lo * 2).ok()?;
    if cp < start_code {
        return Some(0);
    }

    let id_delta = read_i16_be(sub, id_delta_off + lo * 2).ok()?;
    let range_off_pos = id_range_off + lo * 2;
    let id_range_val = read_u16_be(sub, range_off_pos).ok()?;

    if id_range_val == 0 {
        Some((cp as i32 + id_delta as i32) as u16)
    } else {
        let glyph_addr = range_off_pos
            .checked_add(id_range_val as usize)?
            .checked_add((cp - start_code) as usize * 2)?;
        let raw_gid = read_u16_be(sub, glyph_addr).ok()?;
        if raw_gid == 0 {
            Some(0)
        } else {
            Some((raw_gid as i32 + id_delta as i32) as u16)
        }
    }
}

fn lookup_cmap12(sub: &[u8], cp: u32) -> Option<u32> {
    if sub.len() < 16 {
        return None;
    }
    let num_groups = read_u32_be(sub, 12).ok()? as usize;
    if 16usize.checked_add(num_groups * 12)? > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = num_groups;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let end_cp = read_u32_be(sub, 16 + mid * 12 + 4).ok()?;
        if end_cp < cp {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= num_groups {
        return Some(0);
    }

    let off = 16 + lo * 12;
    let start_cp = read_u32_be(sub, off).ok()?;
    let end_cp = read_u32_be(sub, off + 4).ok()?;
    let start_gid = read_u32_be(sub, off + 8).ok()?;
    if cp >= start_cp && cp <= end_cp {
        Some(start_gid.wrapping_add(cp - start_cp))
    } else {
        Some(0)
    }
}

fn read_u16_be(data: &[u8], off: usize) -> Result<u16, &'static str> {
    let end = off.checked_add(2).ok_or("u16 read overflow")?;
    if end > data.len() {
        return Err("truncated u16");
    }
    Ok(u16::from_be_bytes([data[off], data[off + 1]]))
}

fn read_i16_be(data: &[u8], off: usize) -> Result<i16, &'static str> {
    Ok(read_u16_be(data, off)? as i16)
}

fn read_u32_be(data: &[u8], off: usize) -> Result<u32, &'static str> {
    let end = off.checked_add(4).ok_or("u32 read overflow")?;
    if end > data.len() {
        return Err("truncated u32");
    }
    Ok(u32::from_be_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn read_f2dot14(data: &[u8], off: usize) -> Result<f32, &'static str> {
    Ok(read_i16_be(data, off)? as f32 / 16384.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_non_empty(glyph: &[u8; 16]) -> bool {
        glyph.iter().any(|&row| row != 0)
    }

    #[test]
    fn ascii_printable_non_empty_and_distinct() {
        assert!(!is_non_empty(glyph_bitmap(' ')));
        for code in 0x21u8..=0x7Eu8 {
            let ch = code as char;
            let bm = glyph_bitmap(ch);
            assert!(
                is_non_empty(bm),
                "ASCII glyph for {:?} (0x{:02X}) must not be blank",
                ch,
                code
            );
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "ASCII glyph {:?} fell back to unknown glyph",
                ch
            );
        }
    }

    #[test]
    fn cyrillic_full_alphabet_non_empty_and_distinct_from_fallback() {
        let cyrillic_upper: Vec<char> = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ".chars().collect();
        let cyrillic_lower: Vec<char> = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя".chars().collect();
        assert_eq!(cyrillic_upper.len(), 33);
        assert_eq!(cyrillic_lower.len(), 33);

        for ch in cyrillic_upper.into_iter().chain(cyrillic_lower) {
            let bm = glyph_bitmap(ch);
            assert!(is_non_empty(bm), "Cyrillic glyph {:?} must not be blank", ch);
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "Cyrillic glyph {:?} fell back to unknown glyph",
                ch
            );
        }

        // Visually distinct pairs must have distinct bitmaps.
        assert_ne!(glyph_bitmap('Ш'), glyph_bitmap('Щ'));
        assert_ne!(glyph_bitmap('ш'), glyph_bitmap('щ'));
        assert_ne!(glyph_bitmap('Е'), glyph_bitmap('Ё'));
        assert_ne!(glyph_bitmap('е'), glyph_bitmap('ё'));
        assert_ne!(glyph_bitmap('И'), glyph_bitmap('Й'));
        assert_ne!(glyph_bitmap('и'), glyph_bitmap('й'));
        assert_ne!(glyph_bitmap('Ъ'), glyph_bitmap('Ь'));
        assert_ne!(glyph_bitmap('ъ'), glyph_bitmap('ь'));
        assert_ne!(glyph_bitmap('И'), glyph_bitmap('Н'));
    }

    #[test]
    fn symbols_and_fallback() {
        for ch in ['«', '»', '—', '–', '→', '←', '•'] {
            let bm = glyph_bitmap(ch);
            assert!(is_non_empty(bm), "Symbol {:?} must not be blank", ch);
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "Symbol {:?} fell back to unknown glyph",
                ch
            );
        }

        assert_eq!(
            glyph_bitmap('\u{2603}') as *const [u8; 16],
            &GLYPH_FALLBACK as *const [u8; 16]
        );
        assert!(is_non_empty(glyph_bitmap('\u{2603}')));
    }

    #[test]
    fn measure_text_handles_ascii_and_utf8_cyrillic() {
        assert_eq!(measure_text("", 1), 0.0);
        assert_eq!(measure_text("Hello", 1), 40.0);
        assert_eq!(measure_text("Привет", 1), 48.0);
        assert_eq!(measure_text("Привет!", 2), 7.0 * 16.0);
        assert_eq!(measure_text("ab", 0), 0.0);
    }

    #[test]
    fn system_font_engine_rasterizes_ascii_cyrillic_and_compounds() {
        let mut engine = FontEngine::new();
        assert!(
            engine.has_system_font(),
            "Ubuntu/Linux test environment must discover system TTF fonts"
        );
        assert!(engine.line_height(16.0) >= 16.0);
        assert!(line_height_px(16.0) >= 16.0);

        for ch in ['A', 'Ж', 'я', 'i', 'W', 'Ё'] {
            let g = engine.rasterize_glyph(ch, 16.0, false, false).clone();
            assert!(g.width > 0 && g.height > 0, "glyph {:?} must have non-zero size", ch);
            assert_eq!(g.alpha.len(), (g.width * g.height) as usize);
            assert!(
                g.alpha.iter().any(|&a| a > 0),
                "glyph {:?} must have non-empty alpha coverage",
                ch
            );
            assert!(
                g.alpha.iter().any(|&a| a > 0 && a < 255),
                "glyph {:?} should produce anti-aliased edge coverage",
                ch
            );
        }

        let adv_i = engine.rasterize_glyph('i', 16.0, false, false).advance;
        let adv_w = engine.rasterize_glyph('W', 16.0, false, false).advance;
        assert!(
            adv_i < adv_w,
            "proportional advance('i')={} must be < advance('W')={}",
            adv_i,
            adv_w
        );

        let mono_i = engine.rasterize_glyph('i', 16.0, false, true).advance;
        let mono_w = engine.rasterize_glyph('W', 16.0, false, true).advance;
        assert!(
            (mono_i - mono_w).abs() < 0.01,
            "monospace advances must match: {} vs {}",
            mono_i,
            mono_w
        );

        assert!(
            measure_text_px("WWW", 16.0, false, false)
                > measure_text_px("iii", 16.0, false, false)
        );
        assert!(
            (engine.measure_text("Привет!", 16.0, false, false)
                - measure_text_px("Привет!", 16.0, false, false))
            .abs()
                < 0.001
        );
    }

    #[test]
    fn bold_and_cache_hit_and_bounded_eviction() {
        let mut engine = FontEngine::new();
        let reg_sum: u64 = engine
            .rasterize_glyph('A', 20.0, false, false)
            .alpha
            .iter()
            .map(|&a| a as u64)
            .sum();
        let bold_sum: u64 = engine
            .rasterize_glyph('A', 20.0, true, false)
            .alpha
            .iter()
            .map(|&a| a as u64)
            .sum();
        assert!(bold_sum > reg_sum, "bold glyph must have greater ink coverage");

        let ptr1 = engine.rasterize_glyph('Ж', 18.0, false, false) as *const RasterGlyph;
        let len_before = engine.atlas.len();
        let ptr2 = engine.rasterize_glyph('Ж', 18.0, false, false) as *const RasterGlyph;
        assert_eq!(ptr1, ptr2, "cache hit must return exact cached RasterGlyph");
        assert_eq!(engine.atlas.len(), len_before);

        // Insert > MAX_ATLAS_ENTRIES distinct cache entries and ensure cache stays bounded.
        for size_step in 0..45u32 {
            let size = 10.0 + (size_step as f32) * 0.2;
            for code in 0x21u8..=0x7Eu8 {
                let _ = engine.rasterize_glyph(code as char, size, false, false);
            }
        }
        assert!(engine.atlas.len() <= MAX_ATLAS_ENTRIES);
    }

    #[test]
    fn fallback_only_engine_and_malformed_ttf_safe() {
        let mut fb = FontEngine::fallback_only();
        assert!(!fb.has_system_font());
        assert_eq!(fb.measure_text("Hello", 16.0, false, false), 40.0);
        assert_eq!(fb.line_height(16.0), 20.0);

        let g = fb.rasterize_glyph('Ж', 16.0, false, false).clone();
        assert_eq!(g.width, 8);
        assert_eq!(g.height, 16);
        assert_eq!(g.advance, 8.0);
        assert!(g.alpha.contains(&255));

        let g_sp = fb.rasterize_glyph(' ', 16.0, false, false).clone();
        assert_eq!(g_sp.width, 0);
        assert_eq!(g_sp.advance, 8.0);

        // Malformed TTF inputs must return Err cleanly without panicking.
        assert!(TrueTypeFont::parse(Vec::new()).is_err());
        assert!(TrueTypeFont::parse(b"OTTO\x00\x01\x00\x00\x00\x00\x00\x00".to_vec()).is_err());
        assert!(TrueTypeFont::parse(vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00]).is_err());
    }
}
