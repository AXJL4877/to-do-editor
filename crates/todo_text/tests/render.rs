use std::path::PathBuf;
use todo_core::{builtin_text_styles, PerCharReveal, TextContent, DEFAULT_TEXT_STYLE_ID};
use todo_text::{render, FontLibrary, RasterRequest, TextError};

fn repo_fonts() -> FontLibrary {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    FontLibrary::new([
        root.join("dependencies/fonts"),
        PathBuf::from(r"C:\Windows\Fonts"),
    ])
}

fn reference() -> (TextContent, todo_core::TextStyle) {
    let style = builtin_text_styles()
        .into_iter()
        .find(|s| s.id == DEFAULT_TEXT_STYLE_ID)
        .unwrap();
    let mut content = TextContent::new("钱越来越 不值钱了", DEFAULT_TEXT_STYLE_ID);
    content.layout.font_size = 0.155;
    (content, style)
}

/// Skip when neither the brush font nor its system fallbacks exist on this machine.
fn available(fonts: &FontLibrary, style: &todo_core::TextStyle) -> bool {
    std::iter::once(&style.font.file)
        .chain(&style.font.fallbacks)
        .any(|file| fonts.locate(file).is_some())
}

#[test]
fn reference_style_paints_red_core_and_grey_echo() {
    let mut fonts = repo_fonts();
    let (content, style) = reference();
    if !available(&fonts, &style) {
        eprintln!("skipped: no font for {}", style.font.file);
        return;
    }
    let image = render(
        &mut fonts,
        RasterRequest {
            content: &content,
            style: &style,
            canvas: (1920, 1080),
            reveal: None,
        },
    )
    .unwrap();
    assert_eq!((image.width(), image.height()), (1920, 1080));
    let mut red = 0u32;
    let mut grey = 0u32;
    let (mut min_x, mut max_x) = (u32::MAX, 0);
    for y in (0..1080).step_by(2) {
        for x in (0..1920).step_by(2) {
            let [r, g, b, a] = image.pixel(x, y).unwrap();
            if a < 200 {
                continue;
            }
            if r > 180 && g < 60 && b < 60 {
                red += 1;
            } else if r.abs_diff(g) < 12 && g.abs_diff(b) < 12 && (40..120).contains(&r) {
                grey += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
    }
    assert!(red > 2000, "red ink present: {red}");
    assert!(grey > 2000, "grey echo present: {grey}");
    // The 1.33x echo extends well past the red core on both sides.
    assert!(min_x < 300 && max_x > 1620, "echo span {min_x}..{max_x}");
    // Corners stay transparent.
    assert_eq!(image.pixel(5, 5).unwrap()[3], 0);
    assert_eq!(image.pixel(1915, 1075).unwrap()[3], 0);
}

#[test]
fn per_char_reveal_shows_leading_characters_first() {
    let mut fonts = repo_fonts();
    let (content, style) = reference();
    if !available(&fonts, &style) {
        return;
    }
    let coverage = |reveal: Option<PerCharReveal>, fonts: &mut FontLibrary| {
        let image = render(
            fonts,
            RasterRequest {
                content: &content,
                style: &style,
                canvas: (960, 540),
                reveal,
            },
        )
        .unwrap();
        let half = |range: std::ops::Range<u32>| {
            range
                .flat_map(|x| (0..540).step_by(3).map(move |y| (x, y)))
                .filter(|&(x, y)| image.pixel(x, y).unwrap()[3] > 0)
                .count()
        };
        (half(0..480), half(480..960))
    };
    let early = PerCharReveal {
        time: 0.3,
        duration: 1.0,
        char_duration: 0.2,
    };
    let (left, right) = coverage(Some(early), &mut fonts);
    assert!(left > 0, "first characters visible");
    assert_eq!(right, 0, "later characters still hidden");
    let (full_left, full_right) = coverage(None, &mut fonts);
    assert!(full_left > left && full_right > 0);
}

#[test]
fn whitespace_only_text_is_transparent_and_missing_fonts_fail() {
    let mut fonts = repo_fonts();
    let (mut content, mut style) = reference();
    if available(&fonts, &style) {
        content.text = "  \n ".into();
        let image = render(
            &mut fonts,
            RasterRequest {
                content: &content,
                style: &style,
                canvas: (64, 36),
                reveal: None,
            },
        )
        .unwrap();
        assert!(image.premultiplied_rgba().iter().all(|&v| v == 0));
    }
    style.font.file = "missing.ttf".into();
    style.font.fallbacks.clear();
    let err = render(
        &mut fonts,
        RasterRequest {
            content: &content,
            style: &style,
            canvas: (64, 36),
            reveal: None,
        },
    )
    .err();
    assert_eq!(err, Some(TextError::FontNotFound("missing.ttf".into())));
    assert!(fonts.locate("../Cargo.toml").is_none());
    assert!(fonts.locate(r"C:\Windows\Fonts\msyh.ttc").is_none());
}
