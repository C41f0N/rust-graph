// Packed font catalog: the fonts shipped with the binary, decided at build
// time instead of scanning the device. Each family carries pre-baked SDF atlas
// bytes (see editor::text::serialize_sdf_font_cache) for the style cuts it
// ships; a missing cut stays None and rendering falls back to the upright font.
//
// The atlases are produced from assets/fonts/*.ttf with the dev-time bake CLI:
//   rust-sandbox --dump-font-sdf assets/fonts/<Family>-<cut>.ttf assets/fonts/<name>.sdf.bin
// License texts live next to the TTFs in assets/fonts/. Both fonts below are
// SIL OFL. Family order is the picker order; row 0 doubles as the app default.
pub struct PackedFamily {
    pub name: &'static str,
    pub regular: &'static [u8],
    pub bold: Option<&'static [u8]>,
    pub italic: Option<&'static [u8]>,
}

pub static PACKED_FONTS: &[PackedFamily] = &[
    PackedFamily {
        name: "Adwaita Sans",
        regular: include_bytes!("../assets/fonts/AdwaitaSans-regular.sdf.bin"),
        // Adwaita Sans ships upright and italic single-face files only; the
        // collection has no separate Bold face, so bold falls back to upright.
        bold: None,
        italic: Some(include_bytes!("../assets/fonts/AdwaitaSans-italic.sdf.bin")),
    },
    PackedFamily {
        name: "Fira Code",
        regular: include_bytes!("../assets/fonts/FiraCode-regular.sdf.bin"),
        bold: Some(include_bytes!("../assets/fonts/FiraCode-bold.sdf.bin")),
        // Fira Code ships no italic cut.
        italic: None,
    },
];