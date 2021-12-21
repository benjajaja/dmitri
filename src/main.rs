use breadx::{
    auto::xproto::{InputFocus, SetInputFocusRequest, UngrabKeyRequest},
    keysym_to_key,
    prelude::*,
    DisplayConnection, Event, EventMask, KeyboardState, Window, WindowClass,
};
use getopts::Options;
use gluten_keyboard::Key;
use hex_color::HexColor;
use rust_fuzzy_search::fuzzy_search_best_n;
use std::{
    boxed::Box,
    env,
    error::Error,
    fs,
    io::{self, Write},
    os::unix::prelude::MetadataExt,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

mod text;
use text::{FontRenderer, RunOptions};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("f", "fontname", "set font name", "mono");
    opts.optopt("s", "fontsize", "set font size", "32");
    opts.optopt("m", "margin", "set margin", "7");
    opts.optopt("c", "color", "set color", "#ff8800");
    opts.optopt(
        "p",
        "precise-wheight",
        "set additional wheight of subtext matching",
        "5.0",
    );

    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            panic!("{}", f);
        }
    };
    if matches.opt_present("h") {
        println!("Usage: {} [-f <font name> -s <font size> -m <margin> -c <hex color> -p <precise wheight>]", program);
        return Ok(());
    }
    let options = RunOptions {
        fontname: matches.opt_str("f"),
        fontsize: matches
            .opt_str("s")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(32.0),
        color: matches
            .opt_str("c")
            .and_then(|s| s.parse::<HexColor>().ok())
            .map(|h| (h.r, h.g, h.b))
            .unwrap_or((255, 127, 0)),
        margin: matches
            .opt_str("m")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(7),
        precise_wheight: matches
            .opt_str("p")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(5.0),
    };

    let mut conn = DisplayConnection::create(None, None)?;

    let root = conn.default_screen().root;
    let root_geometry = root.geometry_immediate(&mut conn)?;

    let mut params: WindowParameters = Default::default();
    params.background_pixel = Some(conn.default_black_pixel());
    params.override_redirect = Some(1);

    let height = options.fontsize + (options.margin * 2) as f32;
    let window = conn.create_window(
        root,                        // parent
        WindowClass::CopyFromParent, // window class
        None,                        // depth (none means inherit from parent)
        None,                        // visual (none means "       "    "    )
        0,                           // x
        0,
        root_geometry.width, // width
        height as _,         // height
        0,                   // border width
        params,              // additional properties
    )?;

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

    match run(&mut conn, window, root, options) {
        Err(err) => {
            eprintln!("Error: {}", err);
            Err(err)
        }
        Ok(output) => {
            if output.len() > 0 {
                println!("{}", output);
                process::Command::new(output).spawn()?;
            }
            Ok(())
        }
    }
}

fn run<Dpy: Display + ?Sized>(
    conn: &mut Dpy,
    window: Window,
    root: Window,
    options: RunOptions,
) -> Result<String, Box<dyn Error>> {
    let gc = conn.create_gc(window, Default::default())?;

    // set up the exit protocol, this ensures the window exits when the "X"
    // button is clicked
    let wm_delete_window = conn.intern_atom_immediate("WM_DELETE_WINDOW".to_owned(), false)?;
    window.set_wm_protocols(conn, &[wm_delete_window])?;

    let keystate = KeyboardState::new(conn)?;

    let mut input = "".to_string();

    let executables = build_path()?;

    let mut matches: Vec<String> = vec![];
    let mut matches_i: Option<usize> = None;

    let geometry = window.geometry_immediate(conn)?;
    let mut font_render = FontRenderer::new(
        conn,
        window,
        geometry.depth,
        geometry.width as _,
        geometry.height as _,
        &options,
    )?;

    loop {
        let ev = match conn.wait_for_event() {
            Ok(ev) => ev,
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
                                None => input,
                                Some(i) => matches.get(i).map(String::to_owned).unwrap_or(input),
                            };
                            return Ok(output);
                        }
                        Key::Tab => {
                            if matches.len() > 1 {
                                match matches_i {
                                    None => {
                                        if !kp.state.shift() {
                                            matches_i = Some(0);
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
                                            if i > 0 && matches.get(i - 1).is_some() {
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
                                matches = search(&input, &executables, options.precise_wheight);
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
                                matches = search(&input, &executables, options.precise_wheight);
                            }
                        }
                    }
                    font_render.render_text(conn, window, gc, &input, &matches, matches_i)?;
                }
            }
            _ => (),
        }
    }
}

fn build_path() -> Result<Vec<String>, Box<dyn Error>> {
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
                    break;
                }
                let pathbuf = entry.path();
                let metadata = fs::metadata(&pathbuf)?;
                let mode = metadata.mode();
                if metadata.is_file() && mode & 0o111 != 0 {
                    executables.push(filename);
                }
            }
        }
    }
    executables.sort();
    Ok(executables)
}

fn search(input: &String, executables: &Vec<String>, precise_wheight: f32) -> Vec<String> {
    if input.len() == 0 {
        return vec![];
    }

    let list = executables
        .iter()
        .map(String::as_ref)
        .collect::<Vec<&str>>();

    let mut res: Vec<(&str, f32)> = fuzzy_search_best_n(input, &list, 20);
    for i in 0..res.len() {
        if let Some(start) = res[i].0.find(input) {
            res[i].1 += (precise_wheight / (start as f32 + precise_wheight)) as f32;
        }
    }
    res.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    return res.iter().map(|(s, _)| String::from(*s)).collect();
}

#[allow(dead_code)]
fn pt() {
    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    println!("{:?}", since_the_epoch);
}
