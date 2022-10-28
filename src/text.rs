use breadx::{
    prelude::*,
    protocol::xproto::{Gcontext, ImageFormat, Screen, VisualClass, Visualid, Window},
};
use font_loader::system_fonts;
use rusttype::{point, Font, Scale, VMetrics};
use std::{boxed::Box, error::Error};
use x11rb::image::{Image, PixelLayout};

pub type Color = (f32, f32, f32);
pub fn color_from_u8(color: (u8, u8, u8)) -> (f32, f32, f32) {
    (
        (((color.0 as u16) << 8) + 0xFF) as f32,
        ((color.1 as u16) << 8) as f32,
        ((color.2 as u16) << 8) as f32,
    )
}

pub struct RunOptions {
    pub fontname: Option<String>,
    pub fontsize: u16,
    pub color: Color,
    pub margin: u16,
    pub precise_wheight: f32,
}

trait FontRenderDest {
    fn set_pixel(&self, x: usize, y: usize, v: f32);
}

pub struct FontRenderer<'a> {
    font: Font<'a>,
    image: Image<'a>,
    width: u16,
    height: u16,
    margin: u16,
    scale: Scale,
    color: Color,
    color_secondary: Color,
    v_metrics: VMetrics,
    pixel_layout: PixelLayout,
}
impl FontRenderer<'_> {
    pub fn new<Dpy: Display + ?Sized>(
        dpy: &mut Dpy,
        depth: u8,
        width: u16,
        height: u16,
        options: &RunOptions,
    ) -> Result<FontRenderer<'static>, Box<dyn Error>> {
        let image = Image::allocate_native(width, height, depth, dpy.setup())?;

        let font = FontRenderer::font(&options.fontname)?;

        let scale = Scale::uniform(options.fontsize as f32);

        let color = options.color;
        let color_secondary = (color.0 / 2., color.1 / 2., color.2 / 2.);

        let v_metrics = font.v_metrics(scale);

        let screen = &dpy.default_screen();
        let pixel_layout = check_visual(screen, screen.root_visual);

        Ok(FontRenderer {
            font,
            image,
            width,
            height,
            margin: options.margin,
            scale,
            color,
            color_secondary,
            v_metrics,
            pixel_layout,
        })
    }

    fn font(fontname: &Option<String>) -> Result<Font<'static>, Box<dyn Error>> {
        let name = match fontname {
            None => "monospace",
            Some(name) => name,
        };

        let property = system_fonts::FontPropertyBuilder::new()
            .monospace()
            .family(name)
            .family("ProFontWindows Nerd Font Mono")
            .build();
        let (font_data, _) =
            system_fonts::get(&property).ok_or("Could not get system fonts property")?;

        let font: Font<'static> = Font::try_from_vec(font_data).expect("Error constructing Font");
        Ok(font)
    }

    pub fn render_text<Dpy: Display + ?Sized>(
        &mut self,
        dpy: &mut Dpy,
        window: Window,
        gc: Gcontext,
        input: &str,
        matches: &[String],
        matches_i: Option<usize>,
    ) -> Result<(), Box<dyn Error>> {
        // turn off checked mode to speed up painting
        // dpy.set_checked(false);

        // clear image
        let data = self.image.data_mut();
        for i in data {
            *i = 0;
        }

        if input.is_empty() {
            self.render_glyphs(0, "_", self.color);
        } else {
            let mut x: u16 = 0;
            let color = if matches_i.is_none() {
                self.color
            } else {
                self.color_secondary
            };
            x = self.render_glyphs(x, input, color);

            for (i, m) in matches.iter().enumerate() {
                x = self.render_glyphs(x, " ", self.color_secondary);
                let color = if let Some(m_i) = matches_i {
                    if m_i == i {
                        self.color
                    } else {
                        self.color_secondary
                    }
                } else {
                    self.color_secondary
                };
                x = self.render_glyphs(x, m, color);
                if x > self.width as _ {
                    break;
                }
            }
        }

        dpy.put_image(
            ImageFormat::Z_PIXMAP,
            window,
            gc,
            self.width,
            self.height,
            0,
            0,
            0,
            self.pixel_layout.depth(),
            &self.image.data(),
        )?;
        dpy.flush()?;

        Ok(())
    }

    fn render_glyphs(&mut self, offset: u16, text: &str, color: Color) -> u16 {
        let glyphs: Vec<_> = self
            .font
            .layout(
                &(text.to_string() + " "),
                self.scale,
                point(0.0, 0.0 + self.v_metrics.ascent),
            )
            .collect();

        let mut next_x = offset;
        for glyph in glyphs {
            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                let mut outside = false;
                let dst_x = self.margin + offset + (bounding_box.min.x as u16);
                let dst_y = self.margin + (bounding_box.min.y as u16);
                let max_x = self.width - self.margin * 2;
                glyph.draw(|p_x, p_y, v| {
                    let x = dst_x + p_x as u16;
                    let y = dst_y + p_y as u16;
                    if x < max_x {
                        let pixel = self.pixel_layout.encode((
                            (color.0 * v) as u16,
                            (color.1 * v) as u16,
                            (color.2 * v) as u16,
                        ));
                        self.image.put_pixel(x, y, pixel);
                    } else {
                        outside = true;
                    }
                });
                if outside {
                    break;
                } else {
                    next_x = offset + bounding_box.max.x as u16;
                }
            } else {
                next_x = offset + glyph.position().x as u16;
            }
        }
        next_x
    }
}

/// Check that the given visual is "as expected" (pixel values are 0xRRGGBB with RR/GG/BB being the
/// colors). Otherwise, this exits the process.
fn check_visual(screen: &Screen, id: Visualid) -> PixelLayout {
    // Find the information about the visual and at the same time check its depth.
    let visual_info = screen.allowed_depths.iter().find_map(|depth| {
        let info = depth.visuals.iter().find(|depth| depth.visual_id == id);
        info.map(|info| (depth.depth, info))
    });
    let (depth, visual_type) = match visual_info {
        Some(info) => info,
        None => {
            eprintln!("Did not find the root visual's description?!");
            std::process::exit(1);
        }
    };
    // Check that the pixels have red/green/blue components that we can set directly.
    match visual_type.class {
        VisualClass::TRUE_COLOR | VisualClass::DIRECT_COLOR => {}
        _ => {
            eprintln!(
                "The root visual is not true / direct color, but {:?}",
                visual_type,
            );
            std::process::exit(1);
        }
    }
    let result = PixelLayout::from_visual_type(*visual_type)
        .expect("The server sent a malformed visual type");
    assert_eq!(result.depth(), depth);
    result
}
