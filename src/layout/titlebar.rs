//! Window titlebar for stacking mode.
//!
//! The titlebar shows the application icon and the window title on the left, and the minimize,
//! maximize and close buttons on the right. It is drawn with cairo into a texture, which is only
//! redrawn when its contents change.

use std::cell::RefCell;
use std::sync::Arc;

use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{EllipsizeMode, FontDescription};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Logical, Point, Size, Transform};

use super::titlebar_icon::IconImage;
use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};
use crate::utils::ResizeEdge;

pub const HEIGHT: f64 = 32.;
const BUTTON_W: f64 = 46.;
const PADDING: f64 = 12.;
const FONT: &str = "sans 12px";
const GLYPH: f64 = 10.;
const ICON: f64 = 16.;
const ICON_GAP: f64 = 8.;

const BG_ACTIVE: [f64; 3] = [0.125, 0.125, 0.125];
const BG_INACTIVE: [f64; 3] = [0.17, 0.17, 0.17];
const TEXT_ACTIVE: [f64; 3] = [1., 1., 1.];
const TEXT_INACTIVE: [f64; 3] = [0.6, 0.6, 0.6];
const HOVER: [f64; 4] = [1., 1., 1., 0.08];
const CLOSE_HOVER: [f64; 3] = [0.77, 0.17, 0.11];

/// Width of the resize area outside the window.
pub const EDGE_OUTSIDE: f64 = 8.;
/// Height of the resize area inside the top of the titlebar.
pub const EDGE_TOP_INSIDE: f64 = 4.;
/// Distance from a corner where an edge resizes in both directions.
const CORNER: f64 = 16.;

/// Part of the titlebar or the window frame under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlebarPart {
    Title,
    Minimize,
    Maximize,
    Close,
    /// Resize area around the window.
    Edge(ResizeEdge),
}

impl TitlebarPart {
    pub fn is_button(self) -> bool {
        matches!(self, Self::Minimize | Self::Maximize | Self::Close)
    }
}

/// Returns the resize edges at (`x`, `y`) in coordinates of a tile of `size`.
///
/// The resize area extends outside the tile and slightly into its top.
pub fn edges_at(size: Size<f64, Logical>, x: f64, y: f64) -> ResizeEdge {
    let inside = x >= 0. && y >= 0. && x < size.w && y < size.h;
    let near = x >= -EDGE_OUTSIDE
        && y >= -EDGE_OUTSIDE
        && x < size.w + EDGE_OUTSIDE
        && y < size.h + EDGE_OUTSIDE;
    if !near || (inside && y >= EDGE_TOP_INSIDE) {
        return ResizeEdge::empty();
    }
    let (cw, ch) = (CORNER.min(size.w / 3.), CORNER.min(size.h / 3.));
    let mut edges = ResizeEdge::empty();
    if x < cw {
        edges |= ResizeEdge::LEFT;
    } else if x >= size.w - cw {
        edges |= ResizeEdge::RIGHT;
    }
    if y < ch {
        edges |= ResizeEdge::TOP;
    } else if y >= size.h - ch {
        edges |= ResizeEdge::BOTTOM;
    }
    edges
}

/// Returns the titlebar text: the window title followed by the application name, unless the title
/// already contains it.
pub fn caption(title: &str, app_name: Option<&str>) -> String {
    match app_name {
        Some(name) if !name.is_empty() && !title.to_lowercase().contains(&name.to_lowercase()) => {
            if title.is_empty() {
                name.to_owned()
            } else {
                format!("{title} - {name}")
            }
        }
        _ => title.to_owned(),
    }
}

/// Everything the titlebar texture depends on.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    width: i32,
    scale: f64,
    title: String,
    active: bool,
    hover: Option<TitlebarPart>,
    /// Icons are shared per application and never change, so compare them by address.
    icon: Option<usize>,
    maximized: bool,
}

#[derive(Debug, Default)]
pub struct Titlebar {
    pub hover: Option<TitlebarPart>,
    cache: RefCell<Option<(Key, TextureBuffer<GlesTexture>)>>,
}

impl Titlebar {
    /// Returns the titlebar part at `x` from the left of a titlebar of `width`.
    pub fn part_at(width: f64, x: f64) -> TitlebarPart {
        let from_right = width - x;
        if from_right < BUTTON_W {
            TitlebarPart::Close
        } else if from_right < BUTTON_W * 2. {
            TitlebarPart::Maximize
        } else if from_right < BUTTON_W * 3. {
            TitlebarPart::Minimize
        } else {
            TitlebarPart::Title
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        location: Point<f64, Logical>,
        width: f64,
        scale: f64,
        title: &str,
        icon: Option<&Arc<IconImage>>,
        active: bool,
        maximized: bool,
        alpha: f32,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        let key = Key {
            width: (width * scale).round() as i32,
            scale,
            title: title.to_owned(),
            active,
            hover: self.hover.filter(|p| p.is_button()),
            icon: icon.map(|i| Arc::as_ptr(i) as usize),
            maximized,
        };

        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let (data, w, h) = draw(&key, icon)
                .map_err(|err| warn!("error drawing titlebar: {err:?}"))
                .ok()?;
            let buffer = TextureBuffer::from_memory(
                renderer.as_gles_renderer(),
                &data,
                Fourcc::Argb8888,
                (w, h),
                false,
                scale,
                Transform::Normal,
                Vec::new(),
            )
            .map_err(|err| warn!("error uploading titlebar texture: {err:?}"))
            .ok()?;
            *cache = Some((key, buffer));
        }

        let (_, buffer) = cache.as_ref()?;
        let elem = TextureRenderElement::from_texture_buffer(
            buffer.clone(),
            location,
            alpha,
            None,
            None,
            Kind::Unspecified,
        );
        Some(PrimaryGpuTextureRenderElement(elem))
    }
}

fn set_rgb(cr: &cairo::Context, c: [f64; 3]) {
    cr.set_source_rgb(c[0], c[1], c[2]);
}

fn draw(key: &Key, icon: Option<&Arc<IconImage>>) -> anyhow::Result<(Vec<u8>, i32, i32)> {
    let s = key.scale;
    let w = key.width.max(1);
    let h = (HEIGHT * s).round() as i32;

    let surface = ImageSurface::create(cairo::Format::ARgb32, w, h)?;
    let cr = cairo::Context::new(&surface)?;
    cr.scale(s, s);
    let lw = f64::from(w) / s;

    set_rgb(&cr, if key.active { BG_ACTIVE } else { BG_INACTIVE });
    cr.paint()?;

    let button_x = |part: TitlebarPart| match part {
        TitlebarPart::Close => lw - BUTTON_W,
        TitlebarPart::Maximize => lw - BUTTON_W * 2.,
        TitlebarPart::Minimize => lw - BUTTON_W * 3.,
        TitlebarPart::Title | TitlebarPart::Edge(_) => 0.,
    };
    if let Some(part) = key.hover {
        cr.rectangle(button_x(part), 0., BUTTON_W, HEIGHT);
        if part == TitlebarPart::Close {
            set_rgb(&cr, CLOSE_HOVER);
        } else {
            cr.set_source_rgba(HOVER[0], HOVER[1], HOVER[2], HOVER[3]);
        }
        cr.fill()?;
    }

    let text = if key.active {
        TEXT_ACTIVE
    } else {
        TEXT_INACTIVE
    };
    cr.set_line_width(1.);
    let center = |part: TitlebarPart| (button_x(part) + BUTTON_W / 2., HEIGHT / 2.);
    let g = GLYPH / 2.;

    let (cx, cy) = center(TitlebarPart::Minimize);
    set_rgb(&cr, text);
    cr.move_to(cx - g, cy.round() + 0.5);
    cr.line_to(cx + g, cy.round() + 0.5);
    cr.stroke()?;

    let (cx, cy) = center(TitlebarPart::Maximize);
    if key.maximized {
        // Restore: two overlapping squares.
        let (x, y, s2) = ((cx - g).round() + 0.5, (cy - g).round() + 2.5, GLYPH - 3.);
        cr.rectangle(x, y, s2, s2);
        cr.stroke()?;
        cr.move_to(x + 2., y - 2.);
        cr.line_to(x + s2 + 2., y - 2.);
        cr.line_to(x + s2 + 2., y + s2 - 2.);
        cr.stroke()?;
    } else {
        cr.rectangle(
            (cx - g).round() + 0.5,
            (cy - g).round() + 0.5,
            GLYPH - 1.,
            GLYPH - 1.,
        );
        cr.stroke()?;
    }

    let (cx, cy) = center(TitlebarPart::Close);
    if key.hover == Some(TitlebarPart::Close) {
        set_rgb(&cr, [1., 1., 1.]);
    }
    cr.move_to(cx - g, cy - g);
    cr.line_to(cx + g, cy + g);
    cr.move_to(cx + g, cy - g);
    cr.line_to(cx - g, cy + g);
    cr.stroke()?;

    let mut text_x = PADDING;
    if let Some(icon) = icon.and_then(|i| icon_surface(i)) {
        let (iw, ih) = (f64::from(icon.width()), f64::from(icon.height()));
        cr.save()?;
        cr.translate(PADDING, ((HEIGHT - ICON) / 2.).round());
        cr.scale(ICON / iw, ICON / ih);
        cr.set_source_surface(&icon, 0., 0.)?;
        cr.source().set_filter(cairo::Filter::Good);
        cr.paint()?;
        cr.restore()?;
        text_x += ICON + ICON_GAP;
    }

    let room = (button_x(TitlebarPart::Minimize) - text_x - PADDING).max(0.);
    let layout = pangocairo::functions::create_layout(&cr);
    layout.set_font_description(Some(&FontDescription::from_string(FONT)));
    layout.set_ellipsize(EllipsizeMode::End);
    layout.set_width((room * f64::from(pangocairo::pango::SCALE)) as i32);
    layout.set_text(&key.title);
    let (_, text_h) = layout.pixel_size();
    cr.move_to(text_x, ((HEIGHT - f64::from(text_h)) / 2.).round());
    set_rgb(&cr, text);
    pangocairo::functions::show_layout(&cr, &layout);

    drop(cr);
    let data = surface.take_data()?.to_vec();
    Ok((data, w, h))
}

fn icon_surface(icon: &IconImage) -> Option<ImageSurface> {
    ImageSurface::create_for_data(
        icon.data.clone(),
        cairo::Format::ARgb32,
        icon.width,
        icon.height,
        icon.width * 4,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_appends_app_name() {
        assert_eq!(caption("Document", Some("Editor")), "Document - Editor");
        assert_eq!(
            caption("Editor - Document", Some("Editor")),
            "Editor - Document"
        );
        assert_eq!(caption("", Some("Editor")), "Editor");
        assert_eq!(caption("Document", None), "Document");
        assert_eq!(caption("Document", Some("")), "Document");
    }

    #[test]
    fn parts() {
        let w = 500.;
        assert_eq!(Titlebar::part_at(w, 10.), TitlebarPart::Title);
        assert_eq!(Titlebar::part_at(w, w - 1.), TitlebarPart::Close);
        assert_eq!(
            Titlebar::part_at(w, w - BUTTON_W - 1.),
            TitlebarPart::Maximize
        );
        assert_eq!(
            Titlebar::part_at(w, w - BUTTON_W * 2. - 1.),
            TitlebarPart::Minimize
        );
    }

    #[test]
    fn edges() {
        let size = Size::from((400., 300.));
        assert_eq!(edges_at(size, 200., 150.), ResizeEdge::empty());
        assert_eq!(edges_at(size, -4., 150.), ResizeEdge::LEFT);
        assert_eq!(edges_at(size, 404., 150.), ResizeEdge::RIGHT);
        assert_eq!(edges_at(size, 200., 2.), ResizeEdge::TOP);
        assert_eq!(edges_at(size, 200., 304.), ResizeEdge::BOTTOM);
        assert_eq!(edges_at(size, -4., -4.), ResizeEdge::TOP_LEFT);
        assert_eq!(edges_at(size, 200., 10.), ResizeEdge::empty());
        assert_eq!(edges_at(size, 200., -20.), ResizeEdge::empty());
    }
}
