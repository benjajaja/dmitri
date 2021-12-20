use breadx::keysym_to_key;
use breadx::Pixmap;
use breadx::{
    auto::xproto::{InputFocus, SetInputFocusRequest, UngrabKeyRequest},
    prelude::*,
    DisplayConnection, Event, EventMask, Gcontext, Image, ImageFormat, KeyboardState, Window,
    WindowClass,
};
use font_loader::system_fonts;
use getopts::Options;
use gluten_keyboard::Key;
use rs_complete::CompletionTree;
use rusttype::{point, Font, Scale, VMetrics};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::os::unix::prelude::MetadataExt;
use std::process::Command;
use std::{boxed::Box, error::Error, iter, process};

type Color = (i32, i32, i32);

struct FontRender<'a> {
    font: Font<'a>,
    image: Image<Box<[u8]>>,
    pixmap: Pixmap,
    width: u16,
    height: u16,
    scale: Scale,
    color: Color,
    color_secondary: Color,
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
        .ok_or("Could not create Image")?;

        let pixmap = dpy.create_pixmap(window, width as _, height as _, depth)?;

        let font = FontRender::font(fontname)?;

        let scale = Scale::uniform(32.0);

        let color = (0, 200, 50);
        let color_secondary = (0, 100, 25);

        let v_metrics = font.v_metrics(scale);

        return Ok(FontRender {
            font,
            image,
            pixmap,
            width: width as u16,
            height: height as u16,
            scale,
            color,
            color_secondary,
            v_metrics,
        });
    }

    fn font(fontname: Option<String>) -> Result<Font<'static>, Box<dyn Error>> {
        let name: &str = &fontname.unwrap_or("monospace".to_string());

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
        for x in 0..self.width {
            for y in 0..self.height {
                self.image.set_pixel(x as _, y as _, 0);
            }
        }

        if input.len() == 0 {
            self.render_glyphs(0, &"_".to_string(), self.color);
        } else {
            let mut x: u32 = 0;
            x = self.render_glyphs(x, input, self.color);
            if let Some(first) = matches.first() {
                if first.len() > input.len() {
                    let tail = String::from(&first[input.len()..]);
                    x = self.render_glyphs(x, &tail, self.color_secondary);
                }

                for (i, m) in matches[1..].iter().enumerate() {
                    x = self.render_glyphs(x, " ", self.color_secondary);
                    let color = if let Some(m_i) = matches_i {
                        if m_i - 1 == i {
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
            }
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
                // Draw the glyph into the image per-pixel by using the draw closure
                glyph.draw(|x, y, v| {
                    let x = offset as usize + (x as i32 + bounding_box.min.x) as usize;
                    let y = (y as i32 + bounding_box.min.y) as usize;
                    if x < self.width as _ {
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

    let mut conn = DisplayConnection::create(None, None)?;

    let root = conn.default_screen().root;
    let root_geometry = root.geometry_immediate(&mut conn)?;

    let mut params: WindowParameters = Default::default();
    params.background_pixel = Some(conn.default_black_pixel());
    params.override_redirect = Some(1);

    let height = 32;
    let window = conn.create_window(
        root,                        // parent
        WindowClass::CopyFromParent, // window class
        None,                        // depth (none means inherit from parent)
        None,                        // visual (none means "       "    "    )
        0,                           // x
        // (root_geometry.height - height) as i16, // y
        0,
        root_geometry.width, // width
        height,              // height
        0,                   // border width
        params,              // additional properties
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
    let gc = conn.create_gc(window, gc_parameters)?;

    let fontname = matches.opt_str("f");
    match run(&mut conn, window, root, gc, fontname) {
        Err(err) => {
            eprintln!("Error: {}", err);
            Err(err)
        }
        Ok(output) => {
            if output.len() > 0 {
                println!("{}", output);
                Command::new(output).spawn()?;
            }
            Ok(())
        }
    }
}

fn run<Dpy: Display + ?Sized>(
    conn: &mut Dpy,
    window: Window,
    root: Window,
    gc: Gcontext,
    fontname: Option<String>,
) -> Result<String, Box<dyn Error>> {
    // set up the exit protocol, this ensures the window exits when the "X"
    // button is clicked
    let wm_delete_window = conn.intern_atom_immediate("WM_DELETE_WINDOW".to_owned(), false)?;
    window.set_wm_protocols(conn, &[wm_delete_window])?;

    let keystate = KeyboardState::new(conn)?;

    let mut input = "".to_string();

    let completions = build_path()?;

    let mut matches: Vec<String> = vec![];
    let mut matches_i: Option<usize> = None;

    let geometry = window.geometry_immediate(conn)?;
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
            Event::Expose(_) => {
                font_render.render_text(conn, window, gc, &input, &matches, matches_i)?;
            }
            Event::FocusOut(_e) => {
                // println!("Leave: {:?}", e);
                conn.send_request(SetInputFocusRequest {
                    focus: window,
                    revert_to: InputFocus::Parent,
                    ..Default::default()
                })?;
            }
            Event::KeyPress(kp) => {
                let syms = keystate.lookup_keysyms(kp.detail);
                let processed = if syms.is_empty() {
                    None
                } else {
                    keysym_to_key(syms[0])
                };
                if let Some(keycode) = processed {
                    match keycode {
                        Key::Escape => {
                            conn.send_request(UngrabKeyRequest {
                                grab_window: root,
                                ..Default::default()
                            })?;
                            window.unmap(conn)?;
                            window.free(conn)?;
                            return Ok("".to_string());
                        }
                        Key::Enter => {
                            let output: String = match matches_i {
                                None => matches.first().map(String::to_owned).unwrap_or(input),
                                Some(i) => matches.get(i).map(String::to_owned).unwrap_or(input),
                            };
                            return Ok(output);
                        }
                        Key::Tab => {
                            if matches.len() > 1 {
                                match matches_i {
                                    None => {
                                        if !kp.state.shift() {
                                            matches_i = Some(1);
                                        } else {
                                            matches_i = Some(matches.len() - 1);
                                        }
                                    }
                                    Some(i) => {
                                        if !kp.state.shift() {
                                            match matches.get(i + 1) {
                                                Some(_) => matches_i = Some(i + 1),
                                                None => matches_i = None,
                                            }
                                        } else {
                                            if i > 1 && matches.get(i - 1).is_some() {
                                                matches_i = Some(i - 1);
                                            } else {
                                                matches_i = None;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Key::Backspace => {
                            if input.len() > 0 {
                                input = input[0..input.len() - 1].to_string();
                                matches_i = None;
                                matches = search(&input, &completions);
                            }
                        }
                        _ => {
                            if let Some(mut keycode_char) = keycode.as_char() {
                                if !kp.state.shift() {
                                    keycode_char = keycode_char
                                        .to_lowercase()
                                        .next()
                                        .ok_or("lowercase keycode char")?;
                                }
                                input.push(keycode_char);
                                matches_i = None;
                                matches = search(&input, &completions);
                            }
                        }
                    }
                    font_render.render_text(conn, window, gc, &input, &matches, matches_i)?;
                }
                io::stdout().flush()?;
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

fn build_path() -> Result<CompletionTree, Box<dyn Error>> {
    let mut completions = CompletionTree::with_inclusions(&['-', '_', '.']);
    let mut executables: Vec<String> = vec![];

    let path_var = env::var("PATH")?;
    let paths = path_var.split(":");
    for path in paths {
        if let Ok(dir) = fs::read_dir(path) {
            for entry in dir {
                let entry = entry?;

                let os_filename = entry.file_name();
                let filename = os_filename.to_string_lossy().to_string();
                if executables.contains(&filename) {
                    // eprintln!("dupe: {}", filename);
                    break;
                }
                let pathbuf = entry.path();
                let metadata = fs::metadata(&pathbuf)?;
                let mode = metadata.mode();
                if metadata.is_file() && mode & 0o111 != 0 {
                    if path.eq("/usr/bin") {
                        println!("entry: {:?} {}", entry, filename);
                    }
                    completions.insert(&filename);
                    executables.push(filename);
                } else {
                    eprintln!("not file / executable: {:?}", path);
                }
            }
        } else {
            eprintln!("Cannot read dir: {}", path);
        }
    }
    executables.sort();
    Ok(completions)
}

fn search(input: &String, completions: &CompletionTree) -> Vec<String> {
    if input.len() == 0 {
        return vec![];
    }
    return completions.complete(input).unwrap_or(vec![]);
}