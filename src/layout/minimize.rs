//! Window minimize animation: the window shrinks and fades out towards its taskbar button, and
//! comes back out of it when restored.

use anyhow::Context as _;
use smithay::backend::renderer::element::utils::{
    Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};

use crate::animation::Animation;
use crate::render_helpers::offscreen::{OffscreenBuffer, OffscreenData, OffscreenRenderElement};

pub type MinimizeRenderElement =
    RelocateRenderElement<RescaleRenderElement<OffscreenRenderElement>>;

#[derive(Debug)]
pub struct MinimizeAnimation {
    /// Goes from 0 to 1 in both directions.
    anim: Animation,
    minimizing: bool,
    /// Taskbar button relative to the tile position.
    target: Rectangle<f64, Logical>,
    buffer: OffscreenBuffer,
}

impl MinimizeAnimation {
    pub fn new(anim: Animation, minimizing: bool, target: Rectangle<f64, Logical>) -> Self {
        Self {
            anim,
            minimizing,
            target,
            buffer: OffscreenBuffer::default(),
        }
    }

    pub fn is_done(&self) -> bool {
        self.anim.is_done()
    }

    /// Returns 0 when the window is in place and 1 when it is in the taskbar button.
    fn progress(&self) -> f64 {
        let t = self.anim.clamped_value().clamp(0., 1.);
        if self.minimizing {
            t
        } else {
            1. - t
        }
    }

    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        elements: &[impl RenderElement<GlesRenderer>],
        geo_size: Size<f64, Logical>,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> anyhow::Result<(MinimizeRenderElement, OffscreenData)> {
        let p = self.progress();

        let (elem, _sync_point, data) = self
            .buffer
            .render(renderer, scale, elements)
            .context("error rendering to offscreen buffer")?;

        // Shrink down to the button size (but not below 5%) and move the center to the button.
        let end_scale = (self.target.size.w / geo_size.w.max(1.))
            .min(self.target.size.h / geo_size.h.max(1.))
            .max(0.05);
        let s = 1. + (end_scale - 1.) * p;
        let from = geo_size.to_point().downscale(2.);
        let to = self.target.loc + self.target.size.to_point().downscale(2.);
        let center = from + (to - from).upscale(p);
        let top_left = location + center - geo_size.to_point().upscale(s / 2.);

        let elem = elem.with_alpha(alpha * (1. - p as f32).powf(0.7));
        let elem = RescaleRenderElement::from_element(elem, Point::from((0, 0)), s);
        let elem = RelocateRenderElement::from_element(
            elem,
            top_left.to_physical_precise_round(scale),
            Relocate::Relative,
        );
        Ok((elem, data))
    }
}
