//! Share previews for X and other link unfurlers.
//!
//! `GET /share/round/{n}` and `GET /share/win/{proof id}` return a small HTML
//! page whose Open Graph and Twitter card tags point at a 1200×675 PNG
//! (`/share/round/{n}.png`, `/share/win/{id}.png`) rendered here from an SVG
//! template in the Mono style, then send people on to the web app. The cards
//! carry only public round facts (round number, treasure tile, amounts) and
//! the tagline; never an address, a wallet or anything about who dug.
//!
//! Fonts are bundled (Archivo Black and JetBrains Mono, SIL Open Font
//! License, `config/share/fonts/`), so rendering does not depend on the host.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use resvg::{tiny_skia, usvg};
use tokio::sync::Mutex;

use crate::api::AppState;

/// Card size (X `summary_large_image`).
pub const WIDTH: u32 = 1200;
pub const HEIGHT: u32 = 675;
/// The tagline on every card.
pub const TAGLINE: &str = "Everyone sees the field. No one sees you.";

const ARCHIVO_BLACK: &[u8] = include_bytes!("../../../config/share/fonts/ArchivoBlack-Regular.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../../config/share/fonts/JetBrainsMono-wght.ttf");

/// Share pages and cards are only served for settled rounds and stored
/// proofs, which never change: browsers, link unfurlers and the site's edge
/// may keep them a day.
pub const SHARE_CACHE: &str = "public, max-age=86400";

/// Rendered PNGs by cache key; round cards are only rendered once a round is
/// settled and win cards once a proof is stored, so entries never go stale.
#[derive(Default)]
pub struct ShareCache(Mutex<HashMap<String, Arc<Vec<u8>>>>);

fn options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut opt = usvg::Options::default();
        opt.fontdb_mut().load_font_data(ARCHIVO_BLACK.to_vec());
        opt.fontdb_mut().load_font_data(JETBRAINS_MONO.to_vec());
        opt
    })
}

/// Escapes text for SVG and HTML.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Formats zatoshis as ZEC with up to 4 decimals, truncated (never rounds up).
pub fn zec(zat: u64) -> String {
    let whole = zat / 100_000_000;
    let frac = (zat % 100_000_000) / 10_000;
    let text = format!("{whole}.{frac:04}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

/// The ink art on a card's left, generated from the web app's art kit
/// (`node scripts/share-art.mjs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardArt {
    /// Zeb holding the open gold chest (prove-a-win).
    Win,
    /// The open gold chest (a round's result).
    Round,
}

const ART_WIN: &str = include_str!("../../../config/share/art-win.svg");
const ART_ROUND: &str = include_str!("../../../config/share/art-round.svg");
const LOCKUP: &str = include_str!("../../../config/share/lockup.svg");
/// The sparkle pictogram (design system icon set, 24 grid).
const SPARKLE: &str =
    "M12 3v5 M12 16v5 M3 12h5 M16 12h5 M6 6l2.5 2.5 M15.5 15.5L18 18 M18 6l-2.5 2.5 M8.5 15.5L6 18";
/// The chest pictogram.
const CHEST: &str = "M3 10h18v10H3z M3 10c0-4 3-6 9-6s9 2 9 6 M10 11h4v4h-4z M3 14h7 M14 14h7";

/// What a card shows.
pub struct Card {
    pub kicker: String,
    pub headline: String,
    /// Big gold amount (ZEC), if any.
    pub amount: Option<String>,
    pub check_label: String,
    pub art: CardArt,
}

/// Greedy word wrap so each line's estimated width at `size` px fits `max_px`.
fn wrap(text: &str, size: f32, max_px: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if advance(&format!("{line} {word}"), size) <= max_px => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

/// Approximate advance of `text` in Archivo Black at `size` px.
fn advance(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| match c {
            '.' | ',' => 0.3,
            '+' | '-' => 0.6,
            ' ' => 0.27,
            'i' | 'l' | 'I' | 'j' | 't' | 'f' | 'r' => 0.42,
            'm' | 'w' | 'M' | 'W' => 0.98,
            _ => 0.68,
        })
        .sum::<f32>()
        * size
}

/// The amount's largest size; it shrinks until amount and "ZEC" fit the
/// text column (x 600 to 1140), never clipping digits.
const AMOUNT_MAX_SIZE: f32 = 84.0;
const AMOUNT_MIN_SIZE: f32 = 40.0;
/// "ZEC" beside the amount, relative to it (34 at 84).
const AMOUNT_UNIT_SCALE: f32 = 34.0 / 84.0;
/// The column right of the chest icon: 1140 − 652.
const AMOUNT_COLUMN: f32 = 488.0;

/// The font size at which `amount`, a 12 px gap and "ZEC" fit the column.
fn amount_size(amount: &str) -> f32 {
    // The text scales with the size; the 12 px gap does not.
    let text =
        advance(amount, AMOUNT_MAX_SIZE) + advance("ZEC", AMOUNT_MAX_SIZE * AMOUNT_UNIT_SCALE);
    if text + 12.0 <= AMOUNT_COLUMN {
        return AMOUNT_MAX_SIZE;
    }
    (AMOUNT_MAX_SIZE * (AMOUNT_COLUMN - 12.0) / text)
        .floor()
        .max(AMOUNT_MIN_SIZE)
}

/// The card as SVG, in the Mono style of the design's prove-a-win card
/// (10A): ink art on the left in a soft glow (gold: treasure), the round,
/// a big headline, the gold amount, the check link, the tagline and the
/// lockup.
pub fn card_svg(card: &Card) -> String {
    let (art, art_transform) = match card.art {
        CardArt::Win => (ART_WIN, "translate(104 150) scale(3.25)"),
        CardArt::Round => (ART_ROUND, "translate(92 236) scale(3.4)"),
    };
    let (size, lines) = [80.0_f32, 68.0, 58.0]
        .into_iter()
        .map(|size| (size, wrap(&card.headline, size, 540.0)))
        .find(|(_, lines)| lines.len() <= 3)
        .unwrap_or_else(|| (52.0, wrap(&card.headline, 52.0, 540.0)));
    let line_h = size * 0.98;
    let top = 250.0;
    let headline: String = lines
        .iter()
        .enumerate()
        .map(|(k, l)| {
            format!(
                r##"<text x="600" y="{:.0}" font-family="Archivo Black" font-size="{size}" fill="#FFFFFF">{}</text>"##,
                top + line_h * k as f32,
                escape(l)
            )
        })
        .collect();
    let after = top + line_h * (lines.len() as f32 - 1.0);
    let amount = card.amount.as_ref().map_or(String::new(), |a| {
        let y = after + 104.0;
        let size = amount_size(a);
        let unit = size * AMOUNT_UNIT_SCALE;
        let x_unit = 652.0 + advance(a, size) + 12.0;
        format!(
            r##"<g transform="translate(600 {:.0}) scale(1.6)" fill="none" stroke="#E8B54A" stroke-width="2.25" stroke-linecap="square"><path d="{CHEST}"/></g>
<text x="652" y="{:.0}" font-family="Archivo Black" font-size="{size:.0}" fill="#E8B54A">{}</text>
<text x="{x_unit:.0}" y="{:.0}" font-family="Archivo Black" font-size="{unit:.0}" fill="#FFFFFF">ZEC</text>"##,
            y - 52.0,
            y,
            escape(a),
            y,
        )
    });
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">
<defs><radialGradient id="glow"><stop offset="0" stop-color="#E8B54A" stop-opacity=".42"/><stop offset=".5" stop-color="#FFFFFF" stop-opacity=".08"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/></radialGradient></defs>
<rect width="{WIDTH}" height="{HEIGHT}" fill="#0D0D0D"/>
<circle cx="300" cy="410" r="290" fill="url(#glow)"/>
<g transform="{art_transform}">{art}</g>
<g transform="translate(600 136) scale(1.15)" fill="none" stroke="#FFFFFF" stroke-width="2.25" stroke-linecap="square"><path d="{SPARKLE}"/></g>
<text x="640" y="160" font-family="Archivo Black" font-size="24" letter-spacing="4" fill="#FFFFFF">{kicker}</text>
{headline}
{amount}
<line x1="600" y1="560" x2="1140" y2="560" stroke="#FFFFFF" stroke-width="3"/>
<text x="600" y="598" font-family="JetBrains Mono" font-size="16" letter-spacing="3" fill="#A6A6A6">CHECK IT</text>
<text x="600" y="630" font-family="JetBrains Mono" font-size="24" fill="#FFFFFF">{check}</text>
<text x="1012" y="604" text-anchor="end" font-family="JetBrains Mono" font-size="16" fill="#D6D6D6">Everyone sees the field.</text>
<text x="1012" y="628" text-anchor="end" font-family="JetBrains Mono" font-size="16" fill="#D6D6D6">No one sees you.</text>
<g transform="translate(1030 574) scale(0.23) translate(-128 -282)">{LOCKUP}</g>
<rect x="9" y="9" width="1182" height="657" fill="none" stroke="#FFFFFF" stroke-width="6"/>
</svg>"##,
        kicker = escape(&card.kicker),
        check = escape(&card.check_label),
    )
}

/// Renders a card to PNG.
pub fn render_png(card: &Card) -> Result<Vec<u8>, String> {
    let tree = usvg::Tree::from_str(&card_svg(card), options()).map_err(|e| e.to_string())?;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).ok_or("pixmap")?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// The share page: OG and Twitter tags, then a redirect to the app.
pub fn share_html(title: &str, description: &str, image_url: &str, app_url: &str) -> String {
    let (t, d, i, a) = (
        escape(title),
        escape(description),
        escape(image_url),
        escape(app_url),
    );
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>{t}</title>
<meta name="description" content="{d}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Zeek">
<meta property="og:title" content="{t}">
<meta property="og:description" content="{d}">
<meta property="og:image" content="{i}">
<meta property="og:image:width" content="{WIDTH}">
<meta property="og:image:height" content="{HEIGHT}">
<meta property="og:url" content="{a}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{t}">
<meta name="twitter:description" content="{d}">
<meta name="twitter:image" content="{i}">
<meta http-equiv="refresh" content="0; url={a}">
</head><body style="background:#0D0D0D;color:#FFFFFF;font-family:sans-serif">
<p><a href="{a}" style="color:#FFFFFF">{t}</a></p>
</body></html>"#
    )
}

fn split(name: &str) -> (&str, bool) {
    match name.strip_suffix(".png") {
        Some(stem) => (stem, true),
        None => (name, false),
    }
}

async fn png_response(state: &AppState, key: String, card: Card) -> Response {
    let cached = state.share_cache.0.lock().await.get(&key).cloned();
    let bytes = match cached {
        Some(b) => b,
        None => match tokio::task::spawn_blocking(move || render_png(&card)).await {
            Ok(Ok(png)) => {
                let png = Arc::new(png);
                state.share_cache.0.lock().await.insert(key, png.clone());
                png
            }
            Ok(Err(e)) => return internal_error(&e),
            Err(e) => return internal_error(&e.to_string()),
        },
    };
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, SHARE_CACHE),
            (
                header::HeaderName::from_static("cross-origin-resource-policy"),
                "cross-origin",
            ),
        ],
        bytes.as_ref().clone(),
    )
        .into_response()
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, format!("{what} not found")).into_response()
}

/// `GET /share/round/{n}` and `/share/round/{n}.png`.
/// Logs `detail` and answers with a body that reveals nothing internal.
fn internal_error(detail: &str) -> Response {
    tracing::error!(error = %detail, "share page failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error; it has been logged",
    )
        .into_response()
}

pub async fn round_page(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let (stem, png) = split(&name);
    let Ok(round) = stem.parse::<u32>() else {
        return not_found("round");
    };
    let row = match state.store.read(|db| db.settlement(round)) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("settled round"),
        Err(e) => return internal_error(&e.to_string()),
    };
    let headline = match row.outcome {
        zeek_core::settle::RoundOutcome::Drawn { treasure_tile, .. } => {
            format!("The treasure was in tile {}", u32::from(treasure_tile) + 1)
        }
        zeek_core::settle::RoundOutcome::Voided => "Round voided and refunded".to_owned(),
    };
    let prize = row.settlement.distributable;
    let card = Card {
        kicker: format!("ROUND {round}"),
        headline: headline.clone(),
        amount: (prize > 0).then(|| zec(prize)),
        check_label: format!("check round {round}"),
        art: CardArt::Round,
    };
    if png {
        return png_response(&state, format!("round/{round}"), card).await;
    }
    let image = format!("{}/share/round/{round}.png", state.public_url);
    let app = format!("{}/rounds/{round}", state.app_url);
    (
        [(header::CACHE_CONTROL, SHARE_CACHE)],
        Html(share_html(
            &format!("Zeek round {round}: {headline}"),
            TAGLINE,
            &image,
            &app,
        )),
    )
        .into_response()
}

/// `GET /share/win/{id}` and `/share/win/{id}.png`: only for proofs the
/// engine verified and stored.
pub async fn win_page(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let (id, png) = split(&name);
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return not_found("proof");
    }
    let stored = match state.store.read(|db| db.win_proof(id)) {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("proof"),
        Err(e) => return internal_error(&e.to_string()),
    };
    let Ok(proof) = zeek_verify::win::WinProof::decode(&stored.2) else {
        return not_found("proof");
    };
    let round = proof.round;
    let headline = format!("I found the treasure in round {round}");
    let card = Card {
        kicker: format!("ROUND {round} · VERIFIED WIN"),
        headline: headline.clone(),
        amount: Some(format!("+{}", zec(proof.opening.value))),
        check_label: format!("win {}", &id[..8]),
        art: CardArt::Win,
    };
    if png {
        return png_response(&state, format!("win/{id}"), card).await;
    }
    let image = format!("{}/share/win/{id}.png", state.public_url);
    let app = format!("{}/win/{id}", state.app_url);
    (
        [(header::CACHE_CONTROL, SHARE_CACHE)],
        Html(share_html(&headline, TAGLINE, &image, &app)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_amounts_shrink_to_fit_the_card() {
        assert_eq!(amount_size("+0.0225"), AMOUNT_MAX_SIZE);
        for a in ["+12.3456", "123.4567", "+9999.9999"] {
            let size = amount_size(a);
            assert!(size < AMOUNT_MAX_SIZE, "{a}");
            let right = 652.0 + advance(a, size) + 12.0 + advance("ZEC", size * AMOUNT_UNIT_SCALE);
            assert!(right <= 1140.0, "{a}: {right}");
        }
        let card = Card {
            kicker: "ROUND 99999 · VERIFIED WIN".into(),
            headline: "I found the treasure in round 99999".into(),
            amount: Some("+12.3456".into()),
            check_label: "win 78ea3288".into(),
            art: CardArt::Win,
        };
        assert!(
            card_svg(&card).contains(">+12.3456</text>"),
            "every digit is drawn"
        );
        // ZEEK_SHARE_CARD_OUT=<file.png> keeps the picture (docs/examples).
        if let Ok(out) = std::env::var("ZEEK_SHARE_CARD_OUT") {
            std::fs::write(out, render_png(&card).expect("render")).expect("write");
        }
    }

    #[test]
    fn cards_render_and_amounts_truncate() {
        assert_eq!(zec(2_251_250), "0.0225");
        assert_eq!(zec(100_000_000), "1");
        assert_eq!(zec(5_000), "0");
        let card = Card {
            kicker: "ROUND 74 · VERIFIED WIN".into(),
            headline: "I found the treasure in round 74".into(),
            amount: Some("+0.0225".into()),
            check_label: "win 8a4696ec".into(),
            art: CardArt::Win,
        };
        let png = render_png(&card).expect("render");
        assert_eq!(&png[1..4], b"PNG");
        assert!(png.len() > 10_000, "a real picture, {} bytes", png.len());
        let html = share_html("t <x>", "d", "http://e/i.png", "http://a/win/x");
        assert!(html.contains(r#"<meta name="twitter:card" content="summary_large_image">"#));
        assert!(html.contains(r#"<meta property="og:image" content="http://e/i.png">"#));
        assert!(html.contains("t &lt;x&gt;"));
    }
}
