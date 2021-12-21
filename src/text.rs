use breadx::{prelude::*, Gcontext, Image, ImageFormat, Pixmap, Window};
use font_loader::system_fonts;
use rusttype::{point, Font, Scale, VMetrics};
use std::{boxed::Box, error::Error, iter};

pub type Color = (u8, u8, u8);

pub struct RunOptions {
    pub fontname: Option<String>,
    pub fontsize: f32,
    pub color: Color,
    pub margin: u16,
    pub precise_wheight: f32,
}

trait FontRenderDest {
    fn set_pixel(x: usize, y: usize, v: f32);
}

pub struct FontRenderer<'a> {
    font: Font<'a>,
    image: Image<Box<[u8]>>,
    pixmap: Pixmap,
    width: u16,
    height: u16,
    margin: u16,
    scale: Scale,
    color: Color,
    color_secondary: Color,
    v_metrics: VMetrics,
}
impl FontRenderer<'_> {
    pub fn new<Dpy: Display + ?Sized>(
        dpy: &mut Dpy,
        window: Window,
        depth: u8,
        width: u32,
        height: u32,
        options: &RunOptions,
    ) -> Result<FontRenderer<'static>, Box<dyn Error>> {
        let image = Image::new(
            &dpy,
            Some(dpy.default_visual()),
            depth,
            ImageFormat::ZPixmap,
            0,
            create_heap_memory(width, height),
            width as _,
            height as _,
            32,
            None,
        )
        .ok_or("Could not create Image")?;

        let pixmap = dpy.create_pixmap(window, width as _, height as _, depth)?;

        let font = FontRenderer::font(&options.fontname)?;

        let scale = Scale::uniform(options.fontsize);

        let color = options.color;
        let color_secondary = (color.0 / 2, color.1 / 2, color.2 / 2);

        let v_metrics = font.v_metrics(scale);

        return Ok(FontRenderer {
            font,
            image,
            pixmap,
            width: width as u16,
            height: height as u16,
            margin: options.margin,
            scale,
            color,
            color_secondary,
            v_metrics,
        });
    }

    fn font(fontname: &Option<String>) -> Result<Font<'static>, Box<dyn Error>> {
        let name = match fontname {
            None => "monospace",
            Some(name) => name,
        };

        let property = system_fonts::FontPropertyBuilder::new()
            .monospace()
            .family(name)
            .family("ProFontWindows")
            .build();
        let (font_data, _) =
            system_fonts::get(&property).ok_or("Could not get system fonts property")?;

        let font: Font<'static> = Font::try_from_vec(font_data).expect("Error constructing Font");
        return Ok(font);
    }

    pub fn render_text<Dpy: Display + ?Sized>(
        self: &mut Self,
        dpy: &mut Dpy,
        window: Window,
        gc: Gcontext,
        input: &String,
        matches: &Vec<String>,
        matches_i: Option<usize>,
    ) -> Result<(), Box<dyn Error>> {
        // turn off checked mode to speed up painting
        dpy.set_checked(false);

        // clear image
        for i in 0..self.image.data.len() {
            self.image.data[i] = 0;
        }

        if input.len() == 0 {
            self.render_glyphs(0, &"_".to_string(), self.color);
        } else {
            let mut x: u32 = 0;
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
                x = self.render_glyphs(x, &m, color);
                if x > self.width as _ {
                    break;
                }
            }
            // }
        }

        dpy.put_image(
            self.pixmap,
            gc,
            &self.image,
            0,
            0,
            0,
            0,
            self.width as _,
            self.height as _,
        )?;
        dpy.copy_area(
            self.pixmap,
            window,
            gc,
            0,
            0,
            self.width as _,
            self.height as _,
            0,
            0,
        )?;

        dpy.set_checked(true);
        return Ok(());
    }

    fn render_glyphs(self: &mut Self, offset: u32, text: &str, color: Color) -> u32 {
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
                glyph.draw(|x, y, v| {
                    let x = self.margin as usize
                        + offset as usize
                        + (x as i32 + bounding_box.min.x) as usize;
                    let y = self.margin as usize + (y as i32 + bounding_box.min.y) as usize;
                    if x < (self.width - self.margin * 2) as usize {
                        self.image.set_pixel(
                            x,
                            y,
                            rgb(
                                (color.0 as f32 * v) as u8,
                                (color.1 as f32 * v) as u8,
                                (color.2 as f32 * v) as u8,
                            ),
                        );
                        next_x = offset + bounding_box.max.x as u32;
                    } else {
                        outside = true;
                    }
                });
                if outside {
                    break;
                }
            } else {
                next_x = offset + glyph.position().x as u32;
            }
        }
        next_x
    }
}

// Helper function to create a chunk of zeroed heap memory for an image.
#[inline]
fn create_heap_memory(width: u32, height: u32) -> Box<[u8]> {
    iter::repeat(0)
        .take((width * height) as usize * 4)
        .collect()
}
