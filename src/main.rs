use breadx::Pixmap;
use breadx::{
    auto::xproto::{InputFocus, SetInputFocusRequest, UngrabKeyRequest},
    prelude::*,
    DisplayConnection, Event, EventMask, Gcontext, Image, ImageFormat, KeyboardState, Window,
    WindowClass,
};
use font_kit::source::SystemSource;
use font_kit::{family_name::FamilyName, properties::Properties};
use getopts::Options;
use gluten_keyboard::Key;
use rusttype::{point, Font, Scale, VMetrics};
use std::io::{self, Write};
use std::{boxed::Box, error::Error, iter, process};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("f", "", "set font name", "mono");

    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            panic!("{}", f);
        }
    };
    if matches.opt_present("h") {
        println!("Usage: {} [-f <font name>]", program);
        return Ok(());
    }

    // open up the connection
    // note that the connection must be mutable
    let mut conn = DisplayConnection::create(None, None)?;

    // create a 640x400 window.
    let root = conn.default_screen().root;
    let root_geometry = root.geometry_immediate(&mut conn)?;

    let mut params: WindowParameters = Default::default();
    params.background_pixel = Some(conn.default_black_pixel());
    params.override_redirect = Some(1);

    let height = 100;
    let window = conn.create_window(
        root,                                   // parent
        WindowClass::CopyFromParent,            // window class
        None,                                   // depth (none means inherit from parent)
        None,                                   // visual (none means "       "    "    )
        0,                                      // x
        (root_geometry.height - height) as i16, // y
        root_geometry.width,                    // width
        height,                                 // height
        0,                                      // border width
        params,                                 // additional properties
    )?;

    // map the window (e.g. display it) and set its title
    window.set_event_mask(
        &mut conn,
        EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::VISIBILITY_CHANGE
            | EventMask::FOCUS_CHANGE,
    )?;
    window.map(&mut conn)?;
    window.set_title(&mut conn, "Hello World!")?;

    conn.send_request(SetInputFocusRequest {
        focus: window,
        revert_to: InputFocus::Parent,
        ..Default::default()
    })?;

    // set up a graphics context for our window
    let mut gc_parameters: GcParameters = Default::default();
    gc_parameters.foreground = Some(conn.default_black_pixel());
    gc_parameters.graphics_exposures = Some(0);
    gc_parameters.line_width = Some(10);
    let gc = conn.create_gc(window, gc_parameters).unwrap();

    let fontname = matches.opt_str("f");
    return run(&mut conn, window, root, gc, fontname);
}

fn run<Dpy: Display + ?Sized>(
    conn: &mut Dpy,
    window: Window,
    root: Window,
    gc: Gcontext,
    fontname: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // set up the exit protocol, this ensures the window exits when the "X"
    // button is clicked
    let wm_delete_window = conn.intern_atom_immediate("WM_DELETE_WINDOW".to_owned(), false)?;
    window.set_wm_protocols(conn, &[wm_delete_window])?;

    let mut keystate = KeyboardState::new(conn)?;

    let mut input = "".to_string();

    let geometry = window.geometry_immediate(conn).unwrap();
    println!("Window is [{} x {}]", geometry.width, geometry.height);

    let geometry = window.geometry_immediate(conn).unwrap();
    // println!("Window is [{} x {}]", geometry.width, geometry.height);
    let mut font_render = FontRender::new(
        conn,
        window,
        geometry.depth,
        geometry.width as _,
        geometry.height as _,
        fontname,
    )?;

    loop {
        let ev = match conn.wait_for_event() {
            Ok(ev) => ev,
            // Err(ClosedConnection) => break,
            Err(e) => {
                eprintln!("Program closed with error: {:?}", e);
                process::exit(1);
            }
        };

        match ev {
            Event::ClientMessage(cme) => {
                if cme.data.longs()[0] == wm_delete_window.xid {
                    process::exit(0);
                }
            }
            Event::KeyPress(kp) => {
                if let Some(keycode) = keystate.process_keycode(kp.detail, kp.state) {
                    // print!("{:?}", keycode);
                    io::stdout().flush().unwrap();
                    match keycode {
                        Key::Escape => {
                            // cleanup(conn);
                            conn.send_request(UngrabKeyRequest {
                                grab_window: root,
                                ..Default::default()
                            })?;
                            window.unmap(conn)?;
                            window.free(conn)?;
                            return Ok(());
                        }
                        Key::Enter => {
                            println!("{}", input);
                            return Ok(());
                        }
                        Key::Backspace => {
                            if input.len() > 0 {
                                input = input[0..input.len() - 1].to_string();
                            }
                        }
                        _ => {
                            if let Some(mut keycode_char) = keycode.as_char() {
                                if !kp.state.shift() {
                                    keycode_char = keycode_char.to_lowercase().next().unwrap();
                                }
                                input.push(keycode_char);
                            }
                        }
                    }
                    render_text(conn, window, gc, &input, &mut font_render)?;
                }
            }
            Event::Expose(_) => {
                // let geometry = window.geometry_immediate(conn).unwrap();
                // println!("Window is [{} x {}]", geometry.width, geometry.height);
                render_text(conn, window, gc, &input, &mut font_render)?;
            }
            Event::FocusOut(_e) => {
                // println!("Leave: {:?}", e);
                conn.send_request(SetInputFocusRequest {
                    focus: window,
                    revert_to: InputFocus::Parent,
                    ..Default::default()
                })?;
            }
            _ => (),
        }
    }
}

// Helper function to create a chunk of zeroed heap memory for an image.
#[inline]
fn create_heap_memory(width: u32, height: u32) -> Box<[u8]> {
    iter::repeat(0)
        .take((width * height) as usize * 4)
        .collect()
}

struct FontRender<'a> {
    font: Font<'a>,
    image: Image<Box<[u8]>>,
    pixmap: Pixmap,
    width: u16,
    height: u16,
    scale: Scale,
    color: (i32, i32, i32),
    v_metrics: VMetrics,
}
impl FontRender<'_> {
    pub fn new<Dpy: Display + ?Sized>(
        dpy: &mut Dpy,
        window: Window,
        depth: u8,
        width: u32,
        height: u32,
        fontname: Option<String>,
    ) -> Result<FontRender<'static>, Box<dyn Error>> {
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
        .unwrap();

        let pixmap = dpy.create_pixmap(window, width as _, height as _, depth)?;

        let font = FontRender::font(fontname)?;

        let scale = Scale::uniform(32.0);

        // Use a dark red colour
        let color = (0, 200, 50);

        let v_metrics = font.v_metrics(scale);

        return Ok(FontRender {
            font,
            image,
            pixmap,
            width: width as u16,
            height: height as u16,
            scale,
            color,
            v_metrics,
        });
    }

    fn font(fontname: Option<String>) -> Result<Font<'static>, Box<dyn Error>> {
        let name: &str = &fontname.unwrap_or("monospace".to_string());
        let sys_font = SystemSource::new()
            .select_by_postscript_name(name)
            .or_else(|err| {
                eprintln!(
                    "Could not select font by PostScript name \"{}\": {}",
                    name, err
                );
                let properties = Properties::default();
                let families = [
                    FamilyName::Title(name.to_string()),
                    FamilyName::Monospace,
                    FamilyName::SansSerif,
                    FamilyName::Title("mono".to_string()),
                    FamilyName::Title("monospace".to_string()),
                    FamilyName::Title("sans".to_string()),
                ];
                SystemSource::new().select_best_match(&families, &properties)
            })?
            .load()?;
        eprintln!("Selected font: {}", sys_font.family_name());

        let arc = sys_font.copy_font_data().unwrap();

        let new: Vec<u8> = (*arc).clone();
        let font: Font<'static> = Font::try_from_vec(new).expect("Error constructing Font");
        return Ok(font);
    }
}

fn render_text<Dpy: Display + ?Sized>(
    dpy: &mut Dpy,
    window: Window,
    gc: Gcontext,
    input: &String,
    font: &mut FontRender,
) -> Result<(), Box<dyn Error>> {
    // turn off checked mode to speed up painting
    dpy.set_checked(false);

    let text = format!("> {}_", input);

    let glyphs: Vec<_> = font
        .font
        .layout(&text, font.scale, point(20.0, 20.0 + font.v_metrics.ascent))
        .collect();

    // clear image
    for x in 0..font.width {
        for y in 0..font.height {
            font.image.set_pixel(x as _, y as _, 0);
        }
    }

    // Loop through the glyphs in the text, positing each one on a line
    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            // Draw the glyph into the image per-pixel by using the draw closure
            glyph.draw(|x, y, v| {
                font.image.set_pixel(
                    // Offset the position by the glyph bounding box
                    (x as i32 + bounding_box.min.x) as usize,
                    (y as i32 + bounding_box.min.y) as usize,
                    // Turn the coverage into an alpha value
                    // rgb((colour.0)[0], (pixel.0)[1], (pixel.0)[2]),
                    rgb(
                        (font.color.0 as f32 * v) as u8,
                        (font.color.1 as f32 * v) as u8,
                        (font.color.2 as f32 * v) as u8,
                    ),
                )
            });
        }
    }

    dpy.put_image(
        font.pixmap,
        gc,
        &font.image,
        0,
        0,
        0,
        0,
        font.width as _,
        font.height as _,
    )?;
    dpy.copy_area(
        font.pixmap,
        window,
        gc,
        0,
        0,
        font.width as _,
        font.height as _,
        0,
        0,
    )?;

    dpy.set_checked(true);
    return Ok(());
}
