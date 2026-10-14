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

/// What a card shows.
pub struct Card {
    pub kicker: String,
    pub headline: String,
    /// Big gold amount (ZEC), if any.
    pub amount: Option<String>,
    pub check_label: String,
}

/// The card as SVG (Mono: Night ground, White panel lines, Gold only on
/// treasure amounts).
pub fn card_svg(card: &Card) -> String {
    let amount = card.amount.as_ref().map_or(String::new(), |a| {
        format!(
            r##"<text x="88" y="470" font-family="Archivo Black" font-size="128" fill="#E8B54A">{}</text>
<text x="{}" y="470" font-family="Archivo Black" font-size="40" fill="#FFFFFF">ZEC</text>"##,
            escape(a),
            // Archivo Black at 128 px: digits and signs ≈ 0.72 em, the dot ≈ 0.33 em.
            100 + a.chars().map(|c| if c == '.' { 42 } else { 92 }).sum::<usize>(),
        )
    });
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">
<rect width="{WIDTH}" height="{HEIGHT}" fill="#0D0D0D"/>
<rect x="40" y="40" width="1120" height="595" fill="none" stroke="#FFFFFF" stroke-width="3"/>
<path d="M88 96 h58 l-40 46 h40 v14 h-62 l40 -46 h-36 z" fill="#FFFFFF"/>
<text x="166" y="140" font-family="Archivo Black" font-size="40" fill="#FFFFFF">Zeek</text>
<text x="88" y="236" font-family="JetBrains Mono" font-size="26" letter-spacing="4" fill="#A6A6A6">{kicker}</text>
<text x="88" y="318" font-family="Archivo Black" font-size="60" fill="#FFFFFF">{headline}</text>
{amount}
<line x1="88" y1="532" x2="1112" y2="532" stroke="#3A3A3A" stroke-width="1.5"/>
<text x="88" y="586" font-family="JetBrains Mono" font-size="24" fill="#D6D6D6">CHECK IT · {check}</text>
<text x="1112" y="586" text-anchor="end" font-family="Archivo Black" font-size="24" fill="#FFFFFF">{tagline}</text>
</svg>"##,
        kicker = escape(&card.kicker),
        headline = escape(&card.headline),
        check = escape(&card.check_label),
        tagline = escape(TAGLINE),
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
            Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
    };
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
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
pub async fn round_page(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let (stem, png) = split(&name);
    let Ok(round) = stem.parse::<u32>() else {
        return not_found("round");
    };
    let row = match state.db.lock().await.settlement(round) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("settled round"),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
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
    };
    if png {
        return png_response(&state, format!("round/{round}"), card).await;
    }
    let image = format!("{}/share/round/{round}.png", state.public_url);
    let app = format!("{}/rounds/{round}", state.app_url);
    Html(share_html(
        &format!("Zeek round {round}: {headline}"),
        TAGLINE,
        &image,
        &app,
    ))
    .into_response()
}

/// `GET /share/win/{id}` and `/share/win/{id}.png`: only for proofs the
/// engine verified and stored.
pub async fn win_page(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let (id, png) = split(&name);
    if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return not_found("proof");
    }
    let stored = match state.db.lock().await.win_proof(id) {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("proof"),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
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
    };
    if png {
        return png_response(&state, format!("win/{id}"), card).await;
    }
    let image = format!("{}/share/win/{id}.png", state.public_url);
    let app = format!("{}/win/{id}", state.app_url);
    Html(share_html(&headline, TAGLINE, &image, &app)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

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
